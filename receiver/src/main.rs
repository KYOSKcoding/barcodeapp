use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use dioxus::prelude::*;
use iroh::address_lookup::MdnsAddressLookup;
use iroh::{Endpoint, endpoint::presets};
use iroh_tickets::{Ticket, endpoint::EndpointTicket};
use qrcode::QrCode;
use qrcode::render::svg;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use tokio::sync::mpsc;
use tracing::info;

const ALPN: &[u8] = barcode_proto::ALPN;

// ── Shared sync state ─────────────────────────────────────────────────
//
// Maps scanned code strings to their `hidden` (checked) state.
// Written by the Dioxus UI whenever a scan arrives or a checkbox is toggled.
// Read by the iroh task when answering sync polls from the phone.

fn sync_state() -> &'static Mutex<HashMap<String, bool>> {
    static STATE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn sync_state_set(code: &str, hidden: bool) {
    if let Ok(mut map) = sync_state().lock() {
        map.insert(code.to_string(), hidden);
    }
}

fn sync_state_lookup(code: &str) -> bool {
    sync_state()
        .lock()
        .map(|m| *m.get(code).unwrap_or(&false))
        .unwrap_or(false)
}

// ── All-codes list for SYNC_ALL ───────────────────────────────────────
//
// Stores (code, kind_byte) for every scan received, in arrival order.
// Written by the iroh task; read by the same task when handling 0x12 streams.

fn sync_all_list() -> &'static Mutex<Vec<(String, u8)>> {
    static LIST: OnceLock<Mutex<Vec<(String, u8)>>> = OnceLock::new();
    LIST.get_or_init(|| Mutex::new(Vec::new()))
}

fn sync_all_push(code: &str, kind_byte: u8) {
    if let Ok(mut list) = sync_all_list().lock() {
        list.push((code.to_string(), kind_byte));
    }
}

// ── Shop configuration ───────────────────────────────────────────────

#[derive(Clone, Debug)]
struct ShopConfig {
    name: &'static str,
    url: &'static str,
    digit_counts: &'static [usize],
}

const SHOPS: &[ShopConfig] = &[
    ShopConfig {
        name: "REWE",
        url: "https://kartenwelt.rewe.de/rewe-geschenkkarte.html",
        digit_counts: &[13, 39],
    },
    ShopConfig {
        name: "EDEKA",
        url: "https://evci.pin-host.com/evci/#/saldo",
        digit_counts: &[32],
    },
    ShopConfig {
        name: "DM",
        url: "https://www.dm.de/services/services-im-markt/geschenkkarten",
        digit_counts: &[24, 32],
    },
    ShopConfig {
        name: "ALDI",
        url: "https://www.helaba.com/de/aldi/",
        digit_counts: &[20, 36, 38],
    },
    ShopConfig {
        name: "LIDL",
        url: "https://www.lidl.de/c/lidl-geschenkkarten/s10007775",
        digit_counts: &[18, 20, 36, 38],
    },
];

fn detect_shops(code: &str) -> Vec<&'static str> {
    let n = code.chars().filter(|c| c.is_ascii_digit()).count();
    let mut candidates: Vec<&'static str> = SHOPS
        .iter()
        .filter(|s| s.digit_counts.contains(&n))
        .map(|s| s.name)
        .collect();
    candidates.dedup();
    candidates
}

fn shop_url(name: &str) -> Option<&'static str> {
    SHOPS.iter().find(|s| s.name == name).map(|s| s.url)
}

/// Trim/format code for display in UI — mirrors extract_card_number rules.
/// - REWE 39 digits: first 13
/// - ALDI/LIDL 38 digits: drop first 18, keep 20
/// - ALDI/LIDL 36 digits: drop first 18, keep 18
/// - EDEKA 32
/// - Others (incl. DM 24, REWE 13, EDEKA 32): keep as-is
fn format_display_code(code: &str) -> String {
    let digits: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
    let n = digits.len();

    match n {
        39 => digits[..13].to_string(),
        38 => digits[18..].to_string(),
        36 => digits[18..].to_string(),
        32 => format!("{} {}", &digits[11..16], &digits[18..]),
        _ => code.to_string(),
    }
}

// ── Data types ───────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct ScanEntry {
    kind: String,
    code: String,
    extracted_card: Option<String>,  // Card number extracted and validated by barcode-proto
    image_b64: String,
    timestamp: String,
    detected_shops: Vec<&'static str>,
    hidden: bool,
    display_code: String,  // Formatted code for UI display (shop-specific formatting applied)
    manual_count: String,  // user-editable sort number (live input value)
    sort_count: String,    // committed sort key — updated on blur/Enter only
    card_value: String,    // user-editable value note
}

enum IrohEvent {
    Status(String),
    Ticket {
        small_str: String,
        small_svg: String,
        full_str: String,
        full_svg: String,
    },
    Scan(ScanEntry),
}

fn copy_to_clipboard(text: &str) {
    match arboard::Clipboard::new() {
        Ok(mut cb) => {
            if let Err(e) = cb.set_text(text) {
                tracing::warn!("clipboard set failed: {e}");
            }
        }
        Err(e) => tracing::warn!("clipboard init failed: {e}"),
    }
}

// ── Main ─────────────────────────────────────────────────────────────

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "receiver=info,iroh=warn".parse().unwrap()),
        )
        .init();
    dioxus::launch(App);
}

// ── CSS (80s-ish: sharper edges, neon accents, CRT feel) ─────────────

const CSS: &str = r#"
:root { color-scheme: dark; }
* { margin: 0; padding: 0; box-sizing: border-box; }
html, body, #main { height: 100%; }
body {
    background: #0c0c1d; color: #d0d0d0;
    font-family: "Courier New", "Consolas", monospace;
    height: 100vh; display: flex; flex-direction: column; overflow: hidden;
}
#main { display: flex; flex-direction: column; flex: 1; overflow: hidden; }

.header {
    display: flex; align-items: center; justify-content: space-between;
    padding: 10px 20px; background: #111128; border-bottom: 2px solid #ff2d6b;
    flex-shrink: 0;
}
.header h1 { font-size: 18px; color: #ff2d6b; letter-spacing: 2px; text-transform: uppercase; }

.btn {
    padding: 6px 14px; border: 1px solid #444; background: #1a1a3a;
    color: #d0d0d0; border-radius: 2px; cursor: pointer; font-size: 13px;
    font-family: inherit; white-space: nowrap; text-transform: uppercase; letter-spacing: 1px;
}
.btn:hover { background: #2a2a5a; border-color: #666; }
.btn-sm { padding: 3px 8px; font-size: 11px; }
.btn-green { background: #0a3a1a; border-color: #1b6b2b; color: #4ade80; }
.btn-green:hover { background: #1b5e20; }
.btn-orange { background: #3a1a00; border-color: #c65100; color: #ffa040; }
.btn-orange:hover { background: #5a2a00; }
.btn-copy { background: #1a1a3a; border-color: #555; font-size: 11px; padding: 3px 8px; }
.btn-copy:hover { background: #2a2a5a; }
.btn-selected { background: #1b5e20 !important; border-color: #4ade80 !important; color: #4ade80 !important; }

.overlay {
    position: fixed; top: 0; left: 0; right: 0; bottom: 0;
    background: rgba(0,0,0,0.85); display: flex;
    align-items: center; justify-content: center; z-index: 100;
}
.overlay-content {
    background: #111128; padding: 28px; border: 2px solid #ff2d6b;
    text-align: center; max-width: 90vw; max-height: 90vh; overflow: auto;
}
.qr-svg { margin: 14px auto; display: block; }
.qr-svg svg { width: 280px; height: 280px; }
.ticket-str {
    font-family: "Courier New", monospace; font-size: 10px; word-break: break-all;
    max-width: 420px; margin: 10px auto; padding: 8px;
    background: #0c0c1d; border: 1px solid #333; user-select: all; color: #888;
}

.status { font-size: 11px; color: #555; padding: 5px 20px; flex-shrink: 0; border-bottom: 1px solid #1a1a3a; }
.empty-state { text-align: center; padding: 60px; color: #444; font-size: 14px; }

/* Main content */
.content { flex: 1; overflow-y: auto; padding: 14px 18px; }

/* Detail panel */
.detail-panel {
    background: #111128; border: 1px solid #333; padding: 14px; margin-bottom: 14px;
}
.detail-panel h3 { margin-bottom: 8px; color: #ff2d6b; font-size: 14px; letter-spacing: 1px; }
.detail-row { display: flex; align-items: center; gap: 8px; margin-bottom: 6px; }
.detail-row label { font-size: 12px; color: #666; min-width: 55px; }
.detail-row input {
    flex: 1; padding: 4px 8px; background: #0c0c1d; border: 1px solid #333;
    color: #d0d0d0; font-family: "Courier New", monospace; font-size: 13px;
}
.detail-row input:focus { outline: none; border-color: #ff2d6b; }
.shop-buttons { display: flex; gap: 5px; flex-wrap: wrap; margin-top: 5px; }
.shop-detect { font-size: 11px; padding: 2px 8px; margin-bottom: 5px; border: 1px solid #333; }
.shop-detect.found { border-color: #1b6b2b; color: #4ade80; }
.shop-detect.ambiguous { border-color: #c65100; color: #ffa040; }
.shop-detect.none { color: #555; }
.action-row { display: flex; gap: 6px; margin-top: 8px; align-items: center; flex-wrap: wrap; }

/* Table */
table { width: 100%; border-collapse: collapse; margin-top: 10px; }
th {
    background: #111128; padding: 6px 8px; text-align: left;
    font-size: 11px; color: #666; border-bottom: 2px solid #333;
    text-transform: uppercase; letter-spacing: 1px;
}
td { padding: 6px 8px; border-bottom: 1px solid #1a1a3a; font-size: 12px; vertical-align: middle; }
tr:hover { background: #141432; }
tr.selected-row { background: #1a1a4a; }
tr.hidden-row { opacity: 0.4; }
.thumb {
    width: 44px; height: 44px; object-fit: cover; cursor: pointer; border: 1px solid #333;
}
.thumb:hover { border-color: #ff2d6b; }
.check-col { width: 30px; text-align: center; }
.check-col input[type="checkbox"] { cursor: pointer; accent-color: #ff2d6b; }

/* Image viewer */
.modal-image { max-width: 100%; max-height: 80vh; display: block; margin: 0 auto; transition: transform 0.15s; }
.image-viewer {
    background: #111128; border: 1px solid #333; padding: 14px; margin-bottom: 14px; text-align: center;
}
.image-viewer .rotate-bar {
    display: flex; gap: 6px; align-items: center; justify-content: center; margin-top: 8px;
}

.copied-toast { color: #4ade80; font-size: 11px; margin-left: 6px; animation: fadeout 1.5s forwards; }
@keyframes fadeout { 0% { opacity: 1; } 100% { opacity: 0; } }
"#;

// ── Shared state ─────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct AppState {
    scans: Signal<Vec<ScanEntry>>,
    ticket_small: Signal<(String, String)>,
    ticket_full: Signal<(String, String)>,
    show_qr: Signal<bool>,
    show_full_ticket: Signal<bool>,
    selected_image: Signal<Option<usize>>,
    image_rotation: Signal<i32>,
    image_zoom: Signal<f32>,
    status_msg: Signal<String>,
    selected_scan: Signal<Option<usize>>,
    selected_shop: Signal<Option<String>>,
    copy_feedback: Signal<Option<&'static str>>,
    show_hidden: Signal<bool>,
    confirm_delete: Signal<Option<usize>>,
    // TODO: restore once dioxus desktop supports iframe navigation to external URLs
    // see: https://github.com/DioxusLabs/dioxus/issues/3086
    // blocked by hardcoded with_navigation_handler in dioxus-desktop/src/webview.rs
    // iframe_url: Signal<Option<String>>,
}

// ── App ──────────────────────────────────────────────────────────────

#[component]
fn App() -> Element {
    let mut state = AppState {
        scans: use_signal(Vec::new),
        ticket_small: use_signal(|| (String::new(), String::new())),
        ticket_full: use_signal(|| (String::new(), String::new())),
        show_qr: use_signal(|| true),
        show_full_ticket: use_signal(|| false),
        selected_image: use_signal(|| None),
        image_rotation: use_signal(|| 0),
        image_zoom: use_signal(|| 1.0_f32),
        status_msg: use_signal(|| "Starting...".to_string()),
        selected_scan: use_signal(|| None),
        selected_shop: use_signal(|| None),
        copy_feedback: use_signal(|| None),
        show_hidden: use_signal(|| false),
        confirm_delete: use_signal(|| None),
    };
    use_context_provider(|| state);

    let mut rx_holder: Signal<Option<mpsc::UnboundedReceiver<IrohEvent>>> = use_signal(|| None);
    use_hook(move || {
        let (tx, rx) = mpsc::unbounded_channel::<IrohEvent>();
        rx_holder.with_mut(|r| *r = Some(rx));
        tokio::spawn(async move {
            if let Err(e) = run_iroh(tx).await {
                tracing::error!("iroh error: {e:#}");
            }
        });
    });

    use_future(move || async move {
        let mut rx = match rx_holder.with_mut(|r| r.take()) {
            Some(rx) => rx,
            None => return,
        };
        while let Some(event) = rx.recv().await {
            match event {
                IrohEvent::Status(msg) => state.status_msg.set(msg),
                IrohEvent::Ticket {
                    small_str,
                    small_svg,
                    full_str,
                    full_svg,
                } => {
                    state.ticket_small.set((small_str, small_svg));
                    state.ticket_full.set((full_str, full_svg));
                }
                IrohEvent::Scan(entry) => {
                    sync_state_set(&entry.code, false);
                    let idx = state.scans.read().len();
                    let detected = entry.detected_shops.clone();
                    state.scans.write().push(entry);
                    state.selected_scan.set(Some(idx));
                    if detected.len() == 1 {
                        state.selected_shop.set(Some(detected[0].to_string()));
                    } else {
                        state.selected_shop.set(None);
                    }
                }
            }
        }
    });

    rsx! {
        style { {CSS} }
        Header {}
        div { class: "status", "{state.status_msg}" }
        QrOverlay {}
        DeleteConfirmOverlay {}
        div { class: "content",
            DetailPanel {}
            ScanTable {}
        }
    }
}

// ── Delete confirmation overlay ──────────────────────────────────────

#[component]
fn DeleteConfirmOverlay() -> Element {
    let mut state = use_context::<AppState>();
    let idx = match (state.confirm_delete)() {
        Some(i) => i,
        None => return rsx! {},
    };
    let scans = state.scans.read();
    let entry = match scans.get(idx) {
        Some(e) => e,
        None => {
            drop(scans);
            state.confirm_delete.set(None);
            return rsx! {};
        }
    };
    let display = entry.display_code.clone();
    drop(scans);

    rsx! {
        div { class: "overlay",
            onclick: move |_| state.confirm_delete.set(None),
            div { class: "overlay-content", onclick: move |e| e.stop_propagation(),
                h2 { style: "margin-bottom:8px;font-size:16px;color:#ff2d6b;letter-spacing:2px;", "DELETE SCAN?" }
                p { style: "font-family:monospace;font-size:12px;color:#888;margin-bottom:20px;word-break:break-all;", "{display}" }
                div { style: "display:flex;gap:10px;justify-content:center;",
                    button { class: "btn btn-orange",
                        onclick: move |_| {
                            let code = {
                                let s = state.scans.read();
                                s.get(idx).map(|e| e.code.clone())
                            };
                            state.scans.write().remove(idx);
                            if let Some(code) = code {
                                if let Ok(mut list) = sync_all_list().lock() {
                                    if let Some(pos) = list.iter().position(|(c, _)| c == &code) {
                                        list.remove(pos);
                                    }
                                }
                                if let Ok(mut map) = sync_state().lock() {
                                    map.remove(&code);
                                }
                            }
                            // Keep selected_scan index valid
                            if let Some(sel) = (state.selected_scan)() {
                                if sel == idx {
                                    state.selected_scan.set(None);
                                } else if sel > idx {
                                    state.selected_scan.set(Some(sel - 1));
                                }
                            }
                            state.confirm_delete.set(None);
                        },
                        "Delete"
                    }
                    button { class: "btn",
                        onclick: move |_| state.confirm_delete.set(None),
                        "Cancel"
                    }
                }
            }
        }
    }
}

// ── Header ───────────────────────────────────────────────────────────

#[component]
fn Header() -> Element {
    let mut state = use_context::<AppState>();
    let scans = state.scans.read();
    let total = scans.len();
    let hidden = scans.iter().filter(|s| s.hidden).count();
    drop(scans);
    rsx! {
        div { class: "header",
            h1 { "Barcode Receiver" }
            div { style: "display:flex;align-items:center;gap:10px;",
                span { style: "font-size:11px;color:#555;",
                    "{total} scan(s)"
                    if hidden > 0 { " / {hidden} hidden" }
                }
                if hidden > 0 {
                    button {
                        class: if (state.show_hidden)() { "btn btn-sm btn-selected" } else { "btn btn-sm" },
                        onclick: move |_| state.show_hidden.set(!(state.show_hidden)()),
                        if (state.show_hidden)() { "Hide done" } else { "Show done" }
                    }
                }
                button { class: "btn btn-sm",
                    onclick: move |_| state.show_qr.set(!(state.show_qr)()),
                    if (state.show_qr)() { "Hide QR" } else { "Show QR" }
                }
            }
        }
    }
}

// ── QR overlay ───────────────────────────────────────────────────────

#[component]
fn QrOverlay() -> Element {
    let mut state = use_context::<AppState>();
    if !(state.show_qr)() || state.ticket_small.read().1.is_empty() {
        return rsx! {};
    }
    let (ticket_str, qr_svg) = if (state.show_full_ticket)() {
        let t = state.ticket_full.read();
        (t.0.clone(), t.1.clone())
    } else {
        let t = state.ticket_small.read();
        (t.0.clone(), t.1.clone())
    };
    let is_full = (state.show_full_ticket)();
    rsx! {
        div { class: "overlay", onclick: move |_| state.show_qr.set(false),
            div { class: "overlay-content", onclick: move |e| e.stop_propagation(),
                h2 { style: "margin-bottom:8px;font-size:16px;color:#ff2d6b;letter-spacing:2px;", "SCAN TO CONNECT" }
                div { class: "qr-svg", dangerous_inner_html: "{qr_svg}" }
                div { style: "margin:8px 0;",
                    button {
                        class: if is_full { "btn btn-sm btn-selected" } else { "btn btn-sm" },
                        onclick: move |_| state.show_full_ticket.set(!is_full),
                        if is_full { "ID + Addrs" } else { "ID only" }
                    }
                }
                p { style: "font-size:11px;color:#555;margin-bottom:4px;", "Endpoint ticket:" }
                div { class: "ticket-str", "{ticket_str}" }
                button { class: "btn", onclick: move |_| state.show_qr.set(false), "Close" }
            }
        }
    }
}

// ── Detail panel ─────────────────────────────────────────────────────

#[component]
fn DetailPanel() -> Element {
    let mut state = use_context::<AppState>();
    let sel_idx = match (state.selected_scan)() {
        Some(i) => i,
        None => return rsx! {
            div { class: "detail-panel",
                p { style: "color:#444;font-size:13px;", "No scan selected." }
            }
        },
    };
    let scans = state.scans.read();
    let entry = match scans.get(sel_idx) {
        Some(e) => e,
        None => return rsx! {},
    };
    let code_val = entry.code.clone();
    let detected = entry.detected_shops.clone();
    let kind = entry.kind.clone();
    let extracted = entry.extracted_card.clone();
    let timestamp = entry.timestamp.clone();
    let image_b64 = entry.image_b64.clone();
    let raw_digits: String = entry.code.chars().filter(|c| c.is_ascii_digit()).collect();
    let is_32 = raw_digits.len() == 32;
    let edeka_num1 = if is_32 { raw_digits[11..16].to_string() } else { String::new() };
    let edeka_num2 = if is_32 { raw_digits[18..].to_string() } else { String::new() };
    let dm_full = raw_digits.clone();
    let has_image = !image_b64.is_empty();

    // Reset image state when selection changes
    let mut prev_sel = use_signal(|| sel_idx);
    if prev_sel() != sel_idx {
        prev_sel.set(sel_idx);
        state.image_rotation.set(0);
        state.image_zoom.set(1.0);
    }

    let det_label = if detected.len() == 1 {
        format!(">> {}", detected[0])
    } else if detected.len() > 1 {
        format!("?? {} -- choose", detected.join(" / "))
    } else {
        "-- no shop detected".to_string()
    };
    let det_cls = match detected.len() {
        1 => "shop-detect found",
        n if n > 1 => "shop-detect ambiguous",
        _ => "shop-detect none",
    };
    drop(scans);

    rsx! {
        div { class: "detail-panel",
            h3 { "#{sel_idx + 1} {kind}" }
            div { class: "{det_cls}", "{det_label}" }

            // Trimmed card number(s) + Copy button(s)
            if is_32 {
                // 32-digit: two-column DM | EDEKA layout
                div { style: "display:flex;gap:16px;margin-bottom:8px;",
                    div { style: "flex:1;",
                        div { style: "font-size:10px;color:#888;margin-bottom:2px;", "DM" }
                        span { style: "font-family:monospace;font-size:13px;word-break:break-all;", "{dm_full}" }
                        {
                            let dm = dm_full.clone();
                            rsx! {
                                button { class: "btn btn-copy btn-sm",
                                    style: "display:block;margin-top:4px;",
                                    onclick: move |_| {
                                        copy_to_clipboard(&dm);
                                        state.copy_feedback.set(Some("Copied!"));
                                        spawn(async move {
                                            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                                            state.copy_feedback.set(None);
                                        });
                                    },
                                    "Copy DM"
                                }
                            }
                        }
                    }
                    div { style: "flex:1;",
                        div { style: "font-size:10px;color:#888;margin-bottom:2px;", "EDEKA" }
                        span { style: "font-family:monospace;font-size:13px;", "{edeka_num1}  {edeka_num2}" }
                        div { style: "display:flex;gap:4px;margin-top:4px;",
                            {
                                let n1 = edeka_num1.clone();
                                rsx! {
                                    button { class: "btn btn-copy btn-sm",
                                        onclick: move |_| {
                                            copy_to_clipboard(&n1);
                                            state.copy_feedback.set(Some("Copied!"));
                                            spawn(async move {
                                                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                                                state.copy_feedback.set(None);
                                            });
                                        },
                                        "Copy EDEKA 1"
                                    }
                                }
                            }
                            {
                                let n2 = edeka_num2.clone();
                                rsx! {
                                    button { class: "btn btn-copy btn-sm",
                                        onclick: move |_| {
                                            copy_to_clipboard(&n2);
                                            state.copy_feedback.set(Some("Copied!"));
                                            spawn(async move {
                                                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                                                state.copy_feedback.set(None);
                                            });
                                        },
                                        "Copy EDEKA 2"
                                    }
                                }
                            }
                        }
                    }
                }
                if let Some(msg) = (state.copy_feedback)() {
                    span { class: "copied-toast", "{msg}" }
                }
            } else {
                div { class: "detail-row",
                    if let Some(card) = extracted.clone() {
                        span { style: "font-family:monospace;color:#4ade80;font-weight:bold;font-size:14px;", "{card}" }
                        button { class: "btn btn-copy",
                            onclick: move |_| {
                                copy_to_clipboard(&card);
                                state.copy_feedback.set(Some("Copied!"));
                                spawn(async move {
                                    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                                    state.copy_feedback.set(None);
                                });
                            },
                            "Copy"
                        }
                    } else {
                        span { style: "color:#444;font-size:13px;", "--" }
                    }
                    if let Some(msg) = (state.copy_feedback)() {
                        span { class: "copied-toast", "{msg}" }
                    }
                }
            }

            // Timestamp
            div { style: "font-size:11px;color:#555;margin-bottom:8px;", "{timestamp}" }

            // Shop selector
            div { style: "margin-top:0;",
                label { style: "font-size:11px;color:#555;", "Shop:" }
                div { class: "shop-buttons",
                    for shop in SHOPS.iter() {
                        {
                            let name = shop.name.to_string();
                            let name2 = name.clone();
                            let is_sel = state.selected_shop.read().as_deref() == Some(shop.name);
                            let cls = if is_sel { "btn btn-sm btn-selected" }
                                else if detected.contains(&shop.name) { if detected.len() > 1 { "btn btn-sm btn-orange" } else { "btn btn-sm btn-green" } }
                                else { "btn btn-sm" };
                            rsx! {
                                button { class: "{cls}",
                                    onclick: move |_| {
                                        if state.selected_shop.read().as_deref() == Some(&name) {
                                            state.selected_shop.set(None);
                                        } else {
                                            state.selected_shop.set(Some(name.clone()));
                                        }
                                    },
                                    "{name2}"
                                }
                            }
                        }
                    }
                }
            }

            // Inline image viewer
            if has_image {
                {
                    let rot = (state.image_rotation)();
                    let zoom = (state.image_zoom)();
                    let transform = format!("transform:rotate({rot}deg) scale({zoom});");
                    rsx! {
                        div { class: "image-viewer",
                            img {
                                class: "modal-image",
                                src: "data:image/jpeg;base64,{image_b64}",
                                style: "{transform}cursor:pointer;",
                                onclick: move |_| { if (state.image_zoom)() > 1.0 { state.image_zoom.set(1.0); } }
                            }
                            div { class: "rotate-bar",
                                button { class: "btn btn-sm",
                                    onclick: move |_| state.image_rotation.set(((state.image_rotation)() + 180) % 360),
                                    "Rotate 180°"
                                }
                                button { class: "btn btn-sm",
                                    onclick: move |_| {
                                        let z = (state.image_zoom)();
                                        state.image_zoom.set(if z > 1.0 { 1.0 } else { 1.5 });
                                    },
                                    if zoom > 1.0 { "Zoom out" } else { "Zoom 150%" }
                                }
                            }
                        }
                    }
                }
            }

            // Full raw code
            div { style: "margin-top:8px;font-size:11px;color:#555;word-break:break-all;", "{code_val}" }

            // Actions
            div { class: "action-row",
                button { class: "btn btn-green btn-sm",
                    disabled: state.selected_shop.read().is_none(),
                    onclick: move |_| {
                        if let Some(name) = state.selected_shop.read().as_deref()
                            && let Some(url) = shop_url(name)
                            && let Err(e) = opener::open(url)
                        {
                            tracing::warn!("browser: {e}");
                        }
                    },
                    "Open in Browser"
                }
                if let Some(name) = state.selected_shop.read().as_deref() {
                    span { style: "font-size:10px;color:#444;",
                        { shop_url(name).unwrap_or("").to_string() }
                    }
                }
                button { class: "btn btn-sm",
                    style: "margin-left:auto;",
                    onclick: move |_| {
                        for shop in SHOPS.iter() {
                            if let Err(e) = opener::open(shop.url) {
                                tracing::warn!("browser: {e}");
                            }
                            std::thread::sleep(std::time::Duration::from_millis(300));
                        }
                    },
                    "Open All"
                }
            }
        }
    }
}

// ── Image viewer ─────────────────────────────────────────────────────

#[component]
fn ImageViewer() -> Element {
    let mut state = use_context::<AppState>();
    let idx = match (state.selected_image)() {
        Some(i) => i,
        None => return rsx! {},
    };
    let scans = state.scans.read();
    let entry = match scans.get(idx) {
        Some(e) => e,
        None => return rsx! {},
    };
    let src = format!("data:image/jpeg;base64,{}", entry.image_b64);
    let label = format!("{}: {}", entry.kind, entry.code);
    let rot = (state.image_rotation)();
    let zoom = (state.image_zoom)();
    let transform = format!("transform:rotate({rot}deg) scale({zoom});");
    drop(scans);

    rsx! {
        div { class: "image-viewer",
            img { class: "modal-image", src: "{src}", style: "{transform}cursor:pointer;",
                onclick: move |_| { if (state.image_zoom)() > 1.0 { state.image_zoom.set(1.0); } }
            }
            p { style: "margin-top:8px;font-size:11px;color:#555;", "{label}" }
            div { class: "rotate-bar",
                button { class: "btn btn-sm",
                    onclick: move |_| state.image_rotation.set(((state.image_rotation)() + 180) % 360),
                    "Rotate 180°"
                }
                button { class: "btn btn-sm",
                    onclick: move |_| {
                        let z = (state.image_zoom)();
                        state.image_zoom.set(if z > 1.0 { 1.0 } else { 1.5 });
                    },
                    if zoom > 1.0 { "Zoom out" } else { "Zoom 150%" }
                }
                button { class: "btn btn-sm",
                    onclick: move |_| {
                        state.selected_image.set(None);
                        state.image_rotation.set(0);
                        state.image_zoom.set(1.0);
                    },
                    "Close"
                }
            }
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Compare two manual-count strings numerically.
/// Rows with a valid integer come first (ascending); empty/non-numeric go last.
fn cmp_manual_count(a: &str, b: &str) -> std::cmp::Ordering {
    match (a.trim().parse::<i32>(), b.trim().parse::<i32>()) {
        (Ok(na), Ok(nb)) => na.cmp(&nb),
        (Ok(_), Err(_)) => std::cmp::Ordering::Less,
        (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
        (Err(_), Err(_)) => std::cmp::Ordering::Equal,
    }
}

// ── Scan table ───────────────────────────────────────────────────────

#[component]
fn ScanTable() -> Element {
    let state = use_context::<AppState>();
    let show_hidden = (state.show_hidden)();
    let mut entries: Vec<(usize, ScanEntry)> = state
        .scans
        .read()
        .iter()
        .enumerate()
        .filter(|(_, e)| show_hidden || !e.hidden)
        .map(|(i, e)| (i, e.clone()))
        .collect();

    // Sort: unchecked rows by manual # (numeric ascending, unnumbered last),
    // checked rows at the bottom sorted by their # as well.
    entries.sort_by(|(_, a), (_, b)| {
        match (a.hidden, b.hidden) {
            (false, true) => std::cmp::Ordering::Less,
            (true, false) => std::cmp::Ordering::Greater,
            _ => cmp_manual_count(&a.sort_count, &b.sort_count),
        }
    });

    // Always render the table headers so layout is stable before the first scan.
    if entries.is_empty() {
        return rsx! {
            table {
                thead { tr {
                    th { class: "check-col", "" }
                    th { "#" }
                    th { "Value" }
                    th { "Code" }
                    th { "Shop" }
                    th { "Img" }
                    th { "Time" }
                }}
                tbody {
                    tr {
                        td {
                            style: "text-align:center;padding:40px;color:#444;font-size:13px;border-bottom:none;",
                            "No scans yet — connect with the phone app to start."
                        }
                    }
                }
            }
        };
    }

    rsx! {
        table {
            thead { tr {
                th { class: "check-col", "" }
                th { "#" }
                th { "Code" }
                th { "Shop" }
                th { "Img" }
                th { "Time" }
            }}
            tbody {
                for (i, entry) in entries {
                    { render_scan_row(state, i, &entry) }
                }
            }
        }
    }
}

fn render_scan_row(mut state: AppState, i: usize, entry: &ScanEntry) -> Element {
    let src = format!("data:image/jpeg;base64,{}", entry.image_b64);
    let has_image = !entry.image_b64.is_empty();
    let is_selected = (state.selected_scan)() == Some(i);
    let is_hidden = entry.hidden;
    let row_cls = if is_selected && is_hidden {
        "selected-row hidden-row"
    } else if is_selected {
        "selected-row"
    } else if is_hidden {
        "hidden-row"
    } else {
        ""
    };
    let shop_str = if entry.detected_shops.is_empty() {
        "\u{2014}".to_string()
    } else {
        entry.detected_shops.join("/")
    };
    rsx! {
        tr { key: "{i}", class: "{row_cls}", style: "cursor:pointer;",
            onclick: move |_| {
                state.selected_scan.set(Some(i));
                let shops = &state.scans.read()[i].detected_shops;
                if shops.len() == 1 {
                    state.selected_shop.set(Some(shops[0].to_string()));
                } else {
                    state.selected_shop.set(None);
                }
            },
            oncontextmenu: move |e| {
                e.prevent_default();
                state.confirm_delete.set(Some(i));
            },
            td { class: "check-col",
                input {
                    r#type: "checkbox",
                    checked: is_hidden,
                    onclick: move |e| {
                        e.stop_propagation();
                        let new_hidden = !is_hidden;
                        let code = state.scans.read()[i].code.clone();
                        sync_state_set(&code, new_hidden);
                        state.scans.write()[i].hidden = new_hidden;
                    },
                }
            }
            td {
                {
                    let mc = entry.manual_count.clone();
                    rsx! {
                        input {
                            r#type: "text",
                            value: "{mc}",
                            placeholder: "-",
                            maxlength: "3",
                            style: "width:36px;background:transparent;border:none;border-bottom:1px solid #444;color:inherit;font-size:inherit;text-align:center;outline:none;",
                            onclick: move |e| e.stop_propagation(),
                            oninput: move |e| {
                                // Update displayed value immediately but don't re-sort yet.
                                state.scans.write()[i].manual_count = e.value();
                            },
                            onblur: move |_| {
                                // Commit sort key when focus leaves the field.
                                let val = { state.scans.read()[i].manual_count.clone() };
                                state.scans.write()[i].sort_count = val;
                            },
                            onkeydown: move |e| {
                                if e.key().to_string() == "Enter" {
                                    let val = { state.scans.read()[i].manual_count.clone() };
                                    state.scans.write()[i].sort_count = val;
                                }
                            },
                        }
                    }
                }
            }
            td {
                {
                    let cv = entry.card_value.clone();
                    rsx! {
                        input {
                            r#type: "text",
                            value: "{cv}",
                            placeholder: "—",
                            maxlength: "8",
                            style: "width:52px;background:transparent;border:none;border-bottom:1px solid #444;color:inherit;font-size:inherit;text-align:right;outline:none;",
                            onclick: move |e| e.stop_propagation(),
                            oninput: move |e| { state.scans.write()[i].card_value = e.value(); }
                        }
                    }
                }
            }
            td { style: "font-family:inherit;max-width:220px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;",
                {
                    let dc = entry.display_code.clone();
                    let dc_copy = dc.clone();
                    rsx! {
                        div { style: "display:flex;align-items:center;gap:4px;",
                            span { style: "overflow:hidden;text-overflow:ellipsis;white-space:nowrap;flex:1;", "{dc}" }
                            button { class: "btn btn-copy btn-sm",
                                onclick: move |e| { e.stop_propagation(); copy_to_clipboard(&dc_copy); },
                                "Copy"
                            }
                        }
                    }
                }
            }
            td {
                if entry.detected_shops.len() > 1 {
                    span { style: "color:#ffa040;", "{shop_str}" }
                } else if entry.detected_shops.len() == 1 {
                    span { style: "color:#4ade80;", "{shop_str}" }
                } else {
                    span { style: "color:#555;", "{shop_str}" }
                }
            }
            td {
                if has_image {
                    img { class: "thumb", src: "{src}",
                        onclick: move |e| {
                            e.stop_propagation();
                            state.selected_image.set(Some(i));
                            state.image_rotation.set(0);
                            state.image_zoom.set(1.0);
                        },
                    }
                } else {
                    span { style: "color:#333;", "--" }
                }
            }
            td { style: "font-size:11px;color:#444;", "{entry.timestamp}" }
        }
    }
}

// ── Iroh background task ─────────────────────────────────────────────

async fn run_iroh(tx: mpsc::UnboundedSender<IrohEvent>) -> anyhow::Result<()> {
    let _ = tx.send(IrohEvent::Status("Binding endpoint...".to_string()));
    let ep = Endpoint::builder(presets::N0)
        .alpns(vec![ALPN.to_vec()])
        .address_lookup(MdnsAddressLookup::builder())
        .bind()
        .await?;

    info!("Endpoint bound, id: {}", ep.id());
    let _ = tx.send(IrohEvent::Status("Waiting for online...".to_string()));
    ep.online().await;

    let addr = ep.addr();
    let small_addr = iroh::EndpointAddr::from(ep.id());
    let small_str = EndpointTicket::new(small_addr).serialize();
    let full_str = EndpointTicket::new(addr).serialize();
    info!("Ticket (compact): {small_str}");
    info!("Ticket (full): {full_str}");

    let make_svg = |s: &str| -> anyhow::Result<String> {
        let code = QrCode::new(s.as_bytes()).map_err(|e| anyhow::anyhow!("QR: {e}"))?;
        Ok(code.render::<svg::Color>().min_dimensions(280, 280).build())
    };
    let _ = tx.send(IrohEvent::Ticket {
        small_svg: make_svg(&small_str)?,
        full_svg: make_svg(&full_str)?,
        small_str,
        full_str,
    });
    let _ = tx.send(IrohEvent::Status(format!("Online -- {}", ep.id())));

    loop {
        let incoming = match ep.accept().await {
            Some(inc) => inc,
            None => break,
        };
        let conn = match incoming.accept() {
            Ok(a) => match a.await {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("conn failed: {e:#}");
                    continue;
                }
            },
            Err(e) => {
                tracing::warn!("accept failed: {e:#}");
                continue;
            }
        };
        info!("Connection from {:?}", conn.remote_id());
        let tx = tx.clone();
        tokio::spawn(async move {
            loop {
                let (mut send, mut recv) = match conn.accept_bi().await {
                    Ok(p) => p,
                    Err(_) => break,
                };

                // Read the routing byte to distinguish scan streams from sync polls.
                let mut type_buf = [0u8; 1];
                if recv.read_exact(&mut type_buf).await.is_err() {
                    continue;
                }

                match type_buf[0] {
                    0x10 => {
                        // Sync poll: phone asking for checked state of its codes.
                        if let Err(e) = barcode_proto::recv_sync_poll(
                            &mut send,
                            &mut recv,
                            sync_state_lookup,
                        )
                        .await
                        {
                            tracing::warn!("sync poll error: {e:#}");
                        }
                    }
                    0x12 => {
                        // Sync all: phone asking for all scanned codes.
                        let entries = sync_all_list()
                            .lock()
                            .map(|v| v.clone())
                            .unwrap_or_default();
                        if let Err(e) = barcode_proto::recv_sync_all(
                            &mut send,
                            &mut recv,
                            &entries,
                            sync_state_lookup,
                        )
                        .await
                        {
                            tracing::warn!("sync all error: {e:#}");
                        }
                    }
                    kind_byte => {
                        // Scan data stream (kind_byte is already the CodeKind byte).
                        match barcode_proto::recv_scan_with_kind(&mut send, &mut recv, kind_byte)
                            .await
                        {
                            Ok(result) => {
                                let image_b64 = if result.image_jpeg.is_empty() {
                                    String::new()
                                } else {
                                    BASE64.encode(&result.image_jpeg)
                                };
                                let detected_shops = detect_shops(&result.code);
                                let display_code = format_display_code(&result.code);
                                let entry = ScanEntry {
                                    kind: result.kind.as_str().to_string(),
                                    code: result.code.clone(),
                                    extracted_card: result.extracted_card.clone(),
                                    image_b64,
                                    timestamp: format_timestamp(),
                                    detected_shops,
                                    hidden: false,
                                    display_code,
                                    manual_count: String::new(),
                                    sort_count: String::new(),
                                    card_value: String::new(),
                                };
                                sync_all_push(&result.code, result.kind as u8);
                                let log_display =
                                    entry.extracted_card.as_ref().unwrap_or(&entry.code);
                                info!(
                                    "Scan: {} - {} (extracted: {})",
                                    entry.kind, entry.code, log_display
                                );
                                let _ = tx.send(IrohEvent::Scan(entry));
                            }
                            Err(e) => {
                                tracing::warn!("recv error: {e:#}");
                                continue;
                            }
                        }
                    }
                }
            }
        });
    }
    Ok(())
}

fn format_timestamp() -> String {
    use std::time::SystemTime;
    let dur = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{h:02}:{m:02}:{s:02}")
}
