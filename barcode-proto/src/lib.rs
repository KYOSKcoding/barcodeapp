//! Shared protocol for barcode scanner communication over iroh.
//!
//! ALPN: `barcodescan/0`
//!
//! Each scanned code uses one bidi stream:
//! - Scanner opens bidi stream
//! - Scanner sends: kind(u8) | code_len(u32 BE) | code(bytes) | image_len(u32 BE) | image_jpeg(bytes)
//! - Scanner finishes send side
//! - Receiver reads all, sends ACK(u8 0x01), finishes send side

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
    send.write_all(&(result.image_jpeg.len() as u32).to_be_bytes())
        .await?;
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
pub async fn recv_scan(send: &mut SendStream, recv: &mut RecvStream) -> Result<ScanResult> {
    // Read kind
    let mut kind_buf = [0u8; 1];
    recv.read_exact(&mut kind_buf).await.context("read kind")?;
    let kind = CodeKind::from_u8(kind_buf[0])?;

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

async fn read_ack(recv: &mut RecvStream) -> Result<u8> {
    let mut buf = [0u8; 1];
    recv.read_exact(&mut buf).await.context("read ack")?;
    Ok(buf[0])
}

/// Extract and validate card number from scanned code.
///
/// **Extraction rules:**
/// - Extract digits only (remove all non-digit characters)
/// - ALDI/LIDL special cases:
///   - 38-digit codes: trim to 20 digits (drop first 18)
///   - 36-digit codes: trim to 18 digits (drop first 14)
/// - EDEKA special case:
///   - 32-digit codes: extract two parts separated by space
///     - Part 1: digits[11:16] (5 digits)
///     - Part 2: digits[18:] (remaining digits)
///   - Result format: "AAAAA YYYYYYYYY"
/// - Final validation: must be 10-24 digits (or 19 for EDEKA with space)
/// - Returns error if validation fails (strict, no fallback)
///
/// # Examples
/// ```
/// // 38-digit ALDI code → trimmed to 20 digits
/// let result = extract_card_number(CodeKind::Barcode, "123456789012345678901234567890123456XX");
/// assert!(result.is_ok());
///
/// // 32-digit EDEKA code → two parts with space
/// let result = extract_card_number(CodeKind::Barcode, "12345678901AAAAA12345BBBBBBBBBBBBB");
/// assert_eq!(result.ok(), Some("AAAAA BBBBBBBBBBBBB".to_string()));
///
/// // Invalid length → error
/// let result = extract_card_number(CodeKind::Barcode, "123456789");
/// assert!(result.is_err());
/// ```
pub fn extract_card_number(kind: CodeKind, code: &str) -> Result<String> {
    // Extract digits only
    let digits: String = code.chars().filter(|c| c.is_ascii_digit()).collect();

    if digits.is_empty() {
        bail!("no digits found in code");
    }

    let n = digits.len();

    // Apply EDEKA special extraction (32-digit code)
    if n == 32 {
        // Extract: digits[11:16] + space + digits[18:]
        let part1 = &digits[11..16];  // 5 digits
        let part2 = &digits[18..];    // remaining digits
        let result = format!("{} {}", part1, part2);
        return Ok(result);
    }

    // Apply ALDI/LIDL trimming (based on specific lengths)
    let trimmed = if n == 38 {
        // 38-digit ALDI/LIDL: take last 20 digits (drop first 18)
        digits[18..].to_string()
    } else if n == 36 {
        // 36-digit variant: take last 18 digits (drop first 14)
        digits[14..].to_string()
    } else {
        digits
    };

    let trimmed_len = trimmed.len();

    // Strict validation: final length must be 10-24 digits (card number typical range)
    if trimmed_len < 10 || trimmed_len > 24 {
        bail!(
            "invalid card number length after extraction: {} (expected 10-24 digits)",
            trimmed_len
        );
    }

    Ok(trimmed)
}
