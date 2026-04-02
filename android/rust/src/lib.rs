#![cfg(target_os = "android")]
//! JNI bridge for the barcode scanner Android app.
//!
//! Exposes a minimal set of functions to Kotlin: connect to a receiver,
//! send scanned barcodes, check connection status, and disconnect.
//! A global tokio runtime drives all async work.

mod logcat;

use std::sync::OnceLock;

use anyhow::{Context, Result};
use barcode_proto::{CodeKind, ScanResult};
use iroh::address_lookup::MdnsAddressLookup;
use iroh::{Endpoint, endpoint::presets};
use iroh_tickets::{endpoint::EndpointTicket, Ticket};
use jni::{
    JNIEnv, JavaVM,
    objects::{JByteArray, JClass, JString},
    sys::{jboolean, jint, jlong},
};
use tokio::runtime::Runtime;
use tracing::info;

// ── Constants ───────────────────────────────────────────────────────

const ALPN: &[u8] = b"barcodescan/0";

const LOGCAT_FILTER: &str = "\
    warn,\
    iroh=debug,\
    barcode_proto=debug,\
    barcode_scanner_android=debug";

// ── Global runtime ──────────────────────────────────────────────────

fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| Runtime::new().expect("failed to create tokio runtime"))
}

// ── Session handle ──────────────────────────────────────────────────

struct SessionHandle {
    endpoint: Endpoint,
    connection: iroh::endpoint::Connection,
}

impl SessionHandle {
    fn into_raw(self) -> jlong {
        Box::into_raw(Box::new(self)) as jlong
    }

    /// # Safety
    /// The pointer must have been created by `into_raw` and not yet freed.
    unsafe fn from_raw(ptr: jlong) -> &'static Self {
        unsafe { &*(ptr as *const Self) }
    }

    /// # Safety
    /// The pointer must have been created by `into_raw` and not yet freed.
    /// After this call the pointer is invalid.
    unsafe fn drop_raw(ptr: jlong) {
        if ptr != 0 {
            unsafe {
                let _ = Box::from_raw(ptr as *mut Self);
            }
        }
    }
}

// ── JNI lifecycle ───────────────────────────────────────────────────

/// Initializes ndk-context and tracing on library load.
///
/// Called automatically by the JVM when `System.loadLibrary` loads this .so.
#[unsafe(no_mangle)]
pub extern "system" fn JNI_OnLoad(vm: JavaVM, _reserved: *mut std::ffi::c_void) -> jint {
    // SAFETY: The JVM guarantees `vm` is valid during JNI_OnLoad.
    unsafe {
        ndk_context::initialize_android_context(
            vm.get_java_vm_pointer().cast(),
            std::ptr::null_mut(),
        );
    }
    let _ = logcat::init(LOGCAT_FILTER);
    info!("barcode-scanner-android JNI loaded");
    jni::sys::JNI_VERSION_1_6
}

// ── JNI functions ───────────────────────────────────────────────────

/// Connect to a remote receiver using an EndpointTicket string.
/// Returns an opaque handle (non-zero on success, 0 on failure).
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_example_barcodescanner_IrohBridge_connect(
    mut env: JNIEnv,
    _class: JClass,
    ticket: JString,
) -> jlong {
    let ticket_str: String = match env.get_string(&ticket) {
        Ok(s) => s.into(),
        Err(e) => {
            tracing::error!("failed to get ticket string: {e}");
            return 0;
        }
    };

    match runtime().block_on(do_connect(ticket_str)) {
        Ok(handle) => {
            info!("connected successfully");
            handle.into_raw()
        }
        Err(e) => {
            tracing::error!("connect failed: {e:#}");
            0
        }
    }
}

async fn do_connect(ticket_str: String) -> Result<SessionHandle> {
    info!("parsing ticket...");
    let ticket = EndpointTicket::deserialize(&ticket_str).context("parse EndpointTicket")?;

    info!("creating endpoint...");
    let ep = Endpoint::builder(presets::N0)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .context("bind endpoint")?;

    // Enable mDNS for local network discovery.
    info!("enabling mDNS...");
    let mdns = MdnsAddressLookup::builder()
        .build(ep.id())
        .context("build mDNS")?;
    ep.address_lookup().context("address_lookup")?.add(mdns);

    let addr = ticket.endpoint_addr().clone();
    info!("connecting to {addr:?}...");
    let conn = ep
        .connect(addr, ALPN)
        .await
        .context("connect to endpoint")?;

    info!("connected!");
    Ok(SessionHandle {
        endpoint: ep,
        connection: conn,
    })
}

/// Send a scanned barcode/QR code over the connection.
/// Returns JNI_TRUE on success, JNI_FALSE on failure.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_example_barcodescanner_IrohBridge_sendScan(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    kind: jint,
    code: JString,
    image_jpeg: JByteArray,
) -> jboolean {
    if handle == 0 {
        tracing::error!("sendScan called with null handle");
        return jni::sys::JNI_FALSE as jboolean;
    }

    let session = unsafe { SessionHandle::from_raw(handle) };

    let code_str: String = match env.get_string(&code) {
        Ok(s) => s.into(),
        Err(e) => {
            tracing::error!("failed to get code string: {e}");
            return jni::sys::JNI_FALSE as jboolean;
        }
    };

    let image_bytes: Vec<u8> = match env.convert_byte_array(&image_jpeg) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("failed to get image bytes: {e}");
            return jni::sys::JNI_FALSE as jboolean;
        }
    };

    let code_kind = match kind {
        0 => CodeKind::Barcode,
        1 => CodeKind::QrCode,
        _ => {
            tracing::warn!("unknown code kind {kind}, defaulting to Barcode");
            CodeKind::Barcode
        }
    };

    let scan = ScanResult {
        kind: code_kind,
        code: code_str,
        image_jpeg: image_bytes,
        extracted_card: None,  // Android just decodes; extraction happens in barcode-proto
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
            jni::sys::JNI_TRUE as jboolean
        }
        Ok(Err(e)) => {
            tracing::error!("send_scan failed: {e:#}");
            jni::sys::JNI_FALSE as jboolean
        }
        Err(_) => {
            tracing::error!("send_scan timed out after 15s");
            jni::sys::JNI_FALSE as jboolean
        }
    }
}

/// Check if the connection is still alive.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_example_barcodescanner_IrohBridge_isConnected(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jboolean {
    if handle == 0 {
        return jni::sys::JNI_FALSE as jboolean;
    }
    let session = unsafe { SessionHandle::from_raw(handle) };
    // Check if the QUIC connection is still open by inspecting its close reason.
    let connected = session.connection.close_reason().is_none();
    if connected {
        jni::sys::JNI_TRUE as jboolean
    } else {
        jni::sys::JNI_FALSE as jboolean
    }
}

/// Disconnect and free the session handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_example_barcodescanner_IrohBridge_disconnect(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    if handle == 0 {
        return;
    }
    info!("disconnecting...");
    let session = unsafe { Box::from_raw(handle as *mut SessionHandle) };
    session.connection.close(0u32.into(), b"bye");
    runtime().block_on(async {
        session.endpoint.close().await;
    });
    info!("disconnected");
}
