//! Shared protocol for barcode scanner communication over iroh.
//!
//! ALPN: `barcodescan/0`
//!
//! Each bidi stream begins with a routing byte:
//!   0x00 / 0x01 — scan stream (kind = Barcode / QR Code)
//!     Scanner sends: kind(u8) | code_len(u32 BE) | code(bytes) | image_len(u32 BE) | image_jpeg(bytes)
//!     Scanner finishes send side
//!     Receiver reads all, sends ACK(u8 0x01), finishes send side
//!   0x10 — sync poll stream
//!     Phone sends: 0x10 | num_codes(u32 BE) | [code_len(u32 BE) | code(bytes)]*
//!     Phone finishes send side
//!     Receiver replies: num_entries(u32 BE) | [code_len(u32 BE) | code(bytes) | checked(u8)]*
//!     Receiver finishes send side
//!   0x12 — sync all stream
//!     Phone sends: 0x12, then finishes send side
//!     Receiver replies: num_entries(u32 BE) | [code_len(u32 BE) | code(bytes) | kind(u8) | checked(u8)]*
//!     Receiver finishes send side

use anyhow::{Context, Result, bail};
use iroh::endpoint::{Connection, RecvStream, SendStream};

pub const ALPN: &[u8] = b"barcodescan/0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CodeKind {
    Barcode = 0,
    QrCode = 1,
}

impl CodeKind {
    pub fn from_u8(v: u8) -> Result<Self> {
        match v {
            0 => Ok(Self::Barcode),
            1 => Ok(Self::QrCode),
            other => bail!("unknown CodeKind: {other}"),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Barcode => "Barcode",
            Self::QrCode => "QR Code",
        }
    }
}

/// A scanned code with optional image and extracted card number.
#[derive(Debug, Clone)]
pub struct ScanResult {
    pub kind: CodeKind,
    pub code: String,
    pub image_jpeg: Vec<u8>,
    /// Card number extracted and validated from code (None if extraction failed)
    pub extracted_card: Option<String>,
}

const ACK: u8 = 0x01;
const SYNC_POLL: u8 = 0x10;
const SYNC_ALL: u8 = 0x12;

/// Scanner side: send a scan result over a bidi stream.
pub async fn send_scan(conn: &Connection, result: &ScanResult) -> Result<()> {
    let (mut send, mut recv) = conn.open_bi().await.context("open bidi stream")?;

    // Write kind
    send.write_all(&[result.kind as u8]).await?;

    // Write code
    let code_bytes = result.code.as_bytes();
    send.write_all(&(code_bytes.len() as u32).to_be_bytes())
        .await?;
    send.write_all(code_bytes).await?;

    // Write image
    send.write_all(&(result.image_jpeg.len() as u32).to_be_bytes()).await?;
    if !result.image_jpeg.is_empty() {
        send.write_all(&result.image_jpeg).await?;
    }

    // Done sending
    send.finish()?;

    // Wait for ACK
    let ack = read_ack(&mut recv).await?;
    if ack != ACK {
        bail!("unexpected ack byte: {ack}");
    }

    Ok(())
}

/// Receiver side: read a scan result from an accepted bidi stream.
/// The stream routing byte has already been consumed by the caller.
pub async fn recv_scan_with_kind(
    send: &mut SendStream,
    recv: &mut RecvStream,
    kind_byte: u8,
) -> Result<ScanResult> {
    let kind = CodeKind::from_u8(kind_byte)?;

    // Read code
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf)
        .await
        .context("read code len")?;
    let code_len = u32::from_be_bytes(len_buf) as usize;
    if code_len > 10_000 {
        bail!("code too large: {code_len}");
    }
    let mut code_buf = vec![0u8; code_len];
    recv.read_exact(&mut code_buf).await.context("read code")?;
    let code = String::from_utf8(code_buf).context("code not utf8")?;

    // Read image
    let mut img_len_buf = [0u8; 4];
    recv.read_exact(&mut img_len_buf)
        .await
        .context("read image len")?;
    let img_len = u32::from_be_bytes(img_len_buf) as usize;
    if img_len > 5_000_000 {
        bail!("image too large: {img_len}");
    }
    let mut image_jpeg = vec![0u8; img_len];
    if img_len > 0 {
        recv.read_exact(&mut image_jpeg)
            .await
            .context("read image")?;
    }

    // Send ACK
    send.write_all(&[ACK]).await?;
    send.finish()?;

    // Extract and validate card number
    let extracted_card = extract_card_number(kind, &code).ok();

    Ok(ScanResult {
        kind,
        code,
        image_jpeg,
        extracted_card,
    })
}

/// Receiver side: read a scan result from an accepted bidi stream.
/// Reads the routing byte itself; prefer `recv_scan_with_kind` when dispatching.
pub async fn recv_scan(send: &mut SendStream, recv: &mut RecvStream) -> Result<ScanResult> {
    let mut kind_buf = [0u8; 1];
    recv.read_exact(&mut kind_buf).await.context("read kind")?;
    recv_scan_with_kind(send, recv, kind_buf[0]).await
}

/// Phone side: send a sync poll and receive the checked state for each code.
/// Opens a new bidi stream, sends the list of codes, and returns
/// `(code, is_checked_on_receiver)` for every code in the same order.
pub async fn send_sync_poll(conn: &Connection, codes: &[String]) -> Result<Vec<(String, bool)>> {
    let (mut send, mut recv) = conn.open_bi().await.context("open sync bidi stream")?;

    // Routing byte
    send.write_all(&[SYNC_POLL]).await?;

    // Number of codes
    send.write_all(&(codes.len() as u32).to_be_bytes()).await?;

    // Each code
    for code in codes {
        let b = code.as_bytes();
        send.write_all(&(b.len() as u32).to_be_bytes()).await?;
        send.write_all(b).await?;
    }
    send.finish()?;

    // Read response
    let mut n_buf = [0u8; 4];
    recv.read_exact(&mut n_buf).await.context("read response count")?;
    let n = u32::from_be_bytes(n_buf) as usize;
    if n > 10_000 {
        bail!("sync response too large: {n}");
    }

    let mut results = Vec::with_capacity(n);
    for _ in 0..n {
        let mut len_buf = [0u8; 4];
        recv.read_exact(&mut len_buf).await.context("read response code len")?;
        let code_len = u32::from_be_bytes(len_buf) as usize;
        if code_len > 10_000 {
            bail!("response code too large: {code_len}");
        }
        let mut code_buf = vec![0u8; code_len];
        recv.read_exact(&mut code_buf).await.context("read response code")?;
        let code = String::from_utf8(code_buf).context("response code not utf8")?;
        let mut checked_buf = [0u8; 1];
        recv.read_exact(&mut checked_buf).await.context("read checked byte")?;
        results.push((code, checked_buf[0] != 0));
    }

    Ok(results)
}

/// Receiver side: handle a sync poll stream.
/// The routing byte (0x10) has already been consumed by the caller.
/// Calls `lookup(code)` for each received code to determine its checked state,
/// then writes the response and finishes the stream.
pub async fn recv_sync_poll<F>(
    send: &mut SendStream,
    recv: &mut RecvStream,
    lookup: F,
) -> Result<()>
where
    F: Fn(&str) -> bool,
{
    // Read number of codes
    let mut n_buf = [0u8; 4];
    recv.read_exact(&mut n_buf).await.context("read poll count")?;
    let n = u32::from_be_bytes(n_buf) as usize;
    if n > 10_000 {
        bail!("too many codes in sync poll: {n}");
    }

    // Read each code
    let mut codes = Vec::with_capacity(n);
    for _ in 0..n {
        let mut len_buf = [0u8; 4];
        recv.read_exact(&mut len_buf).await.context("read poll code len")?;
        let code_len = u32::from_be_bytes(len_buf) as usize;
        if code_len > 10_000 {
            bail!("poll code too large: {code_len}");
        }
        let mut code_buf = vec![0u8; code_len];
        recv.read_exact(&mut code_buf).await.context("read poll code")?;
        let code = String::from_utf8(code_buf).context("poll code not utf8")?;
        codes.push(code);
    }

    // Write response
    send.write_all(&(codes.len() as u32).to_be_bytes()).await?;
    for code in &codes {
        let checked = lookup(code);
        let b = code.as_bytes();
        send.write_all(&(b.len() as u32).to_be_bytes()).await?;
        send.write_all(b).await?;
        send.write_all(&[checked as u8]).await?;
    }
    send.finish()?;

    Ok(())
}

/// Phone side: request all codes known to the receiver.
/// Opens a new bidi stream, sends routing byte 0x12, and returns
/// `(code, kind, is_checked)` for every entry the receiver has seen.
pub async fn send_sync_all(conn: &Connection) -> Result<Vec<(String, CodeKind, bool)>> {
    let (mut send, mut recv) = conn.open_bi().await.context("open sync-all bidi stream")?;

    // Routing byte only — no payload
    send.write_all(&[SYNC_ALL]).await?;
    send.finish()?;

    // Read response
    let mut n_buf = [0u8; 4];
    recv.read_exact(&mut n_buf).await.context("read sync-all count")?;
    let n = u32::from_be_bytes(n_buf) as usize;
    if n > 10_000 {
        bail!("sync-all response too large: {n}");
    }

    let mut results = Vec::with_capacity(n);
    for _ in 0..n {
        let mut len_buf = [0u8; 4];
        recv.read_exact(&mut len_buf).await.context("read sync-all code len")?;
        let code_len = u32::from_be_bytes(len_buf) as usize;
        if code_len > 10_000 {
            bail!("sync-all code too large: {code_len}");
        }
        let mut code_buf = vec![0u8; code_len];
        recv.read_exact(&mut code_buf).await.context("read sync-all code")?;
        let code = String::from_utf8(code_buf).context("sync-all code not utf8")?;
        let mut kind_buf = [0u8; 1];
        recv.read_exact(&mut kind_buf).await.context("read sync-all kind")?;
        let kind = CodeKind::from_u8(kind_buf[0])?;
        let mut checked_buf = [0u8; 1];
        recv.read_exact(&mut checked_buf).await.context("read sync-all checked")?;
        results.push((code, kind, checked_buf[0] != 0));
    }

    Ok(results)
}

/// Receiver side: handle a sync-all stream.
/// The routing byte (0x12) has already been consumed by the caller.
/// `entries` is a slice of `(code, kind_byte)` for every scan the receiver has seen.
/// `lookup_checked` returns true if a code is currently checked (hidden).
pub async fn recv_sync_all(
    send: &mut SendStream,
    _recv: &mut RecvStream,
    entries: &[(String, u8)],
    lookup_checked: impl Fn(&str) -> bool,
) -> Result<()> {
    send.write_all(&(entries.len() as u32).to_be_bytes()).await?;
    for (code, kind_byte) in entries {
        let checked = lookup_checked(code);
        let b = code.as_bytes();
        send.write_all(&(b.len() as u32).to_be_bytes()).await?;
        send.write_all(b).await?;
        send.write_all(&[*kind_byte]).await?;
        send.write_all(&[checked as u8]).await?;
    }
    send.finish()?;
    Ok(())
}

async fn read_ack(recv: &mut RecvStream) -> Result<u8> {
    let mut buf = [0u8; 1];
    recv.read_exact(&mut buf).await.context("read ack")?;
    Ok(buf[0])
}

/// Extract and validate card number from scanned code.
///
/// Rules match the voucher-scanner.py reference implementation:
/// - Digits only (non-digit chars stripped first)
/// - REWE 39-digit barcode  → first 13 digits
/// - ALDI/LIDL 38-digit     → drop first 18, keep last 20
/// - ALDI/LIDL 36-digit     → drop first 18, keep last 18
/// - All other 10–32-digit codes → kept as-is
///   (covers REWE 13, DM 24/32, LIDL 18/20, ALDI 20, EDEKA 16)
pub fn extract_card_number(_kind: CodeKind, code: &str) -> Result<String> {
    let digits: String = code.chars().filter(|c| c.is_ascii_digit()).collect();

    if digits.is_empty() {
        bail!("no digits found in code");
    }

    let n = digits.len();

    let result = match n {
        // REWE 39-digit barcode: card number is the first 13 digits
        39 => digits[..13].to_string(),
        // ALDI/LIDL 38-digit: drop prefix 18, keep last 20
        38 => digits[18..].to_string(),
        // ALDI/LIDL 36-digit: drop prefix 18, keep last 18
        36 => digits[18..].to_string(),
        // Standard range: REWE 13, DM 24/32, LIDL 18/20, ALDI 20, EDEKA 16 – keep as-is
        10..=32 => digits,
        _ => bail!("unrecognised digit count: {n}"),
    };

    Ok(result)
}
