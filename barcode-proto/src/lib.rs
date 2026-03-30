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

/// A scanned code with optional image.
#[derive(Debug, Clone)]
pub struct ScanResult {
    pub kind: CodeKind,
    pub code: String,
    pub image_jpeg: Vec<u8>,
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

    Ok(ScanResult {
        kind,
        code,
        image_jpeg,
    })
}

async fn read_ack(recv: &mut RecvStream) -> Result<u8> {
    let mut buf = [0u8; 1];
    recv.read_exact(&mut buf).await.context("read ack")?;
    Ok(buf[0])
}
