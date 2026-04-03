//! C-FFI bridge for the barcode scanner iOS app.
//!
//! This crate compiles to a static library (`staticlib`) for iOS.
//! It exposes the same logical API as the JNI bridge in android/rust,
//! but uses C calling conventions instead of JNI.
//!
//! All exported functions are safe to call from Swift via a bridging header.
//! Functions that take raw pointers document their safety requirements.

use std::ffi::CStr;
use std::os::raw::{c_char, c_int};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use barcode_proto::{CodeKind, ScanResult, ALPN};
use iroh::{Endpoint, EndpointAddr, endpoint::Connection};
use iroh_tickets::endpoint::EndpointTicket;
use tokio::runtime::Runtime;
use tracing::info;

// ---------------------------------------------------------------------------
// Global Tokio runtime
// ---------------------------------------------------------------------------

fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| Runtime::new().expect("failed to create tokio runtime"))
}

// ---------------------------------------------------------------------------
// Session handle
// ---------------------------------------------------------------------------

struct SessionHandle {
    endpoint: Endpoint,
    connection: Connection,
}

impl SessionHandle {
    fn into_raw(self) -> i64 {
        Box::into_raw(Box::new(self)) as i64
    }

    /// # Safety
    /// `ptr` must have been returned by `into_raw` and must not have been freed.
    unsafe fn from_raw<'a>(ptr: i64) -> &'a Self {
        unsafe { &*(ptr as *const Self) }
    }

    /// # Safety
    /// `ptr` must have been returned by `into_raw` and must not be used afterward.
    unsafe fn drop_raw(ptr: i64) {
        if ptr != 0 {
            unsafe { drop(Box::from_raw(ptr as *mut Self)) };
        }
    }
}

// ---------------------------------------------------------------------------
// Async helpers
// ---------------------------------------------------------------------------

async fn do_connect(ticket_str: String) -> Result<SessionHandle> {
    info!("parsing ticket...");
    let ticket: EndpointTicket = ticket_str.parse().context("parse EndpointTicket")?;

    info!("creating endpoint...");
    let ep = Endpoint::builder(iroh::endpoint::presets::N0)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .context("bind endpoint")?;

    // Note: mDNS (address-lookup-mdns) is intentionally omitted on iOS.
    // Apple requires the com.apple.developer.networking.multicast entitlement
    // for raw multicast sockets. Users must use the full ticket URL.

    let addr: EndpointAddr = ticket.endpoint_addr().clone();
    info!("connecting to {:?}...", addr);
    let conn: Connection = ep.connect(addr, ALPN).await.context("connect to endpoint")?;
    info!("connected!");

    Ok(SessionHandle {
        endpoint: ep,
        connection: conn,
    })
}

// ---------------------------------------------------------------------------
// Exported C-FFI functions
// ---------------------------------------------------------------------------

/// Connects to a remote receiver using an EndpointTicket string.
///
/// Returns an opaque session handle (non-zero on success, 0 on failure).
///
/// # Safety
/// `ticket` must be a valid, null-terminated C string for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn barcode_scanner_connect(ticket: *const c_char) -> i64 {
    if ticket.is_null() {
        tracing::error!("barcode_scanner_connect: null ticket pointer");
        return 0;
    }

    let ticket_str = match unsafe { CStr::from_ptr(ticket) }.to_str() {
        Ok(s) => s.to_owned(),
        Err(e) => {
            tracing::error!("barcode_scanner_connect: invalid UTF-8 in ticket: {e}");
            return 0;
        }
    };

    match runtime().block_on(do_connect(ticket_str)) {
        Ok(handle) => {
            info!("connected successfully");
            handle.into_raw()
        }
        Err(e) => {
            tracing::error!("barcode_scanner_connect failed: {e:#}");
            0
        }
    }
}

/// Sends a scanned barcode/QR code to the connected receiver.
///
/// Returns true on success.
///
/// # Safety
/// - `handle` must be a valid handle returned by `barcode_scanner_connect`.
/// - `code` must be a valid null-terminated C string for the duration of the call.
/// - `image_jpeg` must point to `image_len` valid bytes, or be NULL if `image_len` is 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn barcode_scanner_send_scan(
    handle: i64,
    kind: c_int,
    code: *const c_char,
    image_jpeg: *const u8,
    image_len: usize,
) -> bool {
    if handle == 0 {
        tracing::error!("barcode_scanner_send_scan: null handle");
        return false;
    }
    if code.is_null() {
        tracing::error!("barcode_scanner_send_scan: null code pointer");
        return false;
    }

    let session = unsafe { SessionHandle::from_raw(handle) };

    let code_str = match unsafe { CStr::from_ptr(code) }.to_str() {
        Ok(s) => s.to_owned(),
        Err(e) => {
            tracing::error!("barcode_scanner_send_scan: invalid UTF-8 in code: {e}");
            return false;
        }
    };

    // SAFETY: caller guarantees image_jpeg points to image_len valid bytes.
    let image_bytes: Vec<u8> = if image_jpeg.is_null() || image_len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(image_jpeg, image_len) }.to_vec()
    };

    let code_kind = match kind {
        0 => CodeKind::Barcode,
        1 => CodeKind::QrCode,
        other => {
            tracing::warn!("unknown code kind {other}, defaulting to Barcode");
            CodeKind::Barcode
        }
    };

    let scan = ScanResult {
        kind: code_kind,
        code: code_str,
        image_jpeg: image_bytes,
        extracted_card: None, // extraction happens in barcode-proto on the receiver side
    };

    info!("sending scan: {} '{}'", scan.kind.as_str(), scan.code);

    match runtime().block_on(async {
        tokio::time::timeout(
            std::time::Duration::from_secs(15),
            barcode_proto::send_scan(&session.connection, &scan),
        )
        .await
    }) {
        Ok(Ok(())) => {
            info!("scan sent and ACKed");
            true
        }
        Ok(Err(e)) => {
            tracing::error!("send_scan failed: {e:#}");
            false
        }
        Err(_elapsed) => {
            tracing::error!("send_scan timed out after 15s");
            false
        }
    }
}

/// Returns true if the connection is still alive.
///
/// # Safety
/// `handle` must be a valid handle returned by `barcode_scanner_connect`,
/// or 0 (returns false).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn barcode_scanner_is_connected(handle: i64) -> bool {
    if handle == 0 {
        return false;
    }
    let session = unsafe { SessionHandle::from_raw(handle) };
    session.connection.close_reason().is_none()
}

/// Disconnects and frees the session handle.
///
/// The handle must not be used after this call.
///
/// # Safety
/// `handle` must be a valid handle returned by `barcode_scanner_connect`,
/// or 0 (no-op).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn barcode_scanner_disconnect(handle: i64) {
    if handle == 0 {
        return;
    }
    info!("disconnecting...");
    // Reconstruct Box to get ownership, then close gracefully.
    let session = unsafe { Box::from_raw(handle as *mut SessionHandle) };
    session.connection.close(0u32.into(), b"bye");
    runtime().block_on(async {
        session.endpoint.close().await;
    });
    info!("disconnected");
}
