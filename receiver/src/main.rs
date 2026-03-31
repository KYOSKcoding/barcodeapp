use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use dioxus::prelude::*;
use iroh::address_lookup::MdnsAddressLookup;
use iroh::{Endpoint, endpoint::presets};
use iroh_tickets::{Ticket, endpoint::EndpointTicket};
use qrcode::QrCode;
use qrcode::render::svg;
use tokio::sync::mpsc;
use tracing::info;

const ALPN: &[u8] = barcode_proto::ALPN;

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
.modal-image { max-width: 100%; max-height: 60vh; display: block; margin: 0 auto; transition: transform 0.15s; }
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
    status_msg: Signal<String>,
    selected_scan: Signal<Option<usize>>,
    pin_input: Signal<String>,
    selected_shop: Signal<Option<String>>,
    copy_feedback: Signal<Option<&'static str>>,
    show_hidden: Signal<bool>,
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
        status_msg: use_signal(|| "Starting...".to_string()),
        selected_scan: use_signal(|| None),
        pin_input: use_signal(String::new),
        selected_shop: use_signal(|| None),
        copy_feedback: use_signal(|| None),
        show_hidden: use_signal(|| false),
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
                    let idx = state.scans.read().len();
                    let detected = entry.detected_shops.clone();
                    state.scans.write().push(entry);
                    state.selected_scan.set(Some(idx));
                    state.pin_input.set(String::new());
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
        div { class: "content",
            DetailPanel {}
            ImageViewer {}
            ScanTable {}
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
        None => return rsx! {},
    };
    let scans = state.scans.read();
    let entry = match scans.get(sel_idx) {
        Some(e) => e,
        None => return rsx! {},
    };
    let code_val = entry.code.clone();
    let detected = entry.detected_shops.clone();
    let kind = entry.kind.clone();
    let code_for_copy = code_val.clone();
    let pin_for_copy = state.pin_input.read().clone();
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
            div { class: "detail-row",
                label { "Card:" }
                input { r#type: "text", value: "{code_val}", readonly: true }
                button { class: "btn btn-copy",
                    onclick: move |_| {
                        copy_to_clipboard(&code_for_copy);
                        state.copy_feedback.set(Some("Copied!"));
                        spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                            state.copy_feedback.set(None);
                        });
                    },
                    "Copy"
                }
            }
            div { class: "detail-row",
                label { "PIN:" }
                input {
                    r#type: "text", value: "{state.pin_input}", maxlength: "6",
                    placeholder: "enter pin",
                    oninput: move |e: Event<FormData>| state.pin_input.set(e.value()),
                }
                button { class: "btn btn-copy",
                    onclick: move |_| {
                        let p = pin_for_copy.clone();
                        if !p.is_empty() {
                            copy_to_clipboard(&p);
                            state.copy_feedback.set(Some("PIN copied!"));
                            spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                                state.copy_feedback.set(None);
                            });
                        }
                    },
                    "Copy"
                }
            }
            if let Some(msg) = (state.copy_feedback)() {
                span { class: "copied-toast", "{msg}" }
            }
            div { style: "margin-top:8px;",
                label { style: "font-size:11px;color:#555;", "Shop:" }
                div { class: "shop-buttons",
                    for shop in SHOPS.iter() {
                        {
                            let name = shop.name.to_string();
                            let name2 = name.clone();
                            let is_sel = state.selected_shop.read().as_deref() == Some(shop.name);
                            let cls = if is_sel { "btn btn-sm btn-selected" }
                                else if detected.contains(&shop.name) { "btn btn-sm btn-green" }
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
    let transform = format!("transform:rotate({rot}deg);");
    drop(scans);

    rsx! {
        div { class: "image-viewer",
            img { class: "modal-image", src: "{src}", style: "{transform}" }
            p { style: "margin-top:8px;font-size:11px;color:#555;", "{label}" }
            div { class: "rotate-bar",
                button { class: "btn btn-sm",
                    onclick: move |_| state.image_rotation.set(((state.image_rotation)() - 90) % 360),
                    "< Rot"
                }
                button { class: "btn btn-sm",
                    onclick: move |_| state.image_rotation.set(((state.image_rotation)() + 90) % 360),
                    "Rot >"
                }
                button { class: "btn btn-sm",
                    onclick: move |_| { state.selected_image.set(None); state.image_rotation.set(0); },
                    "Close"
                }
            }
        }
    }
}

// ── Scan table ───────────────────────────────────────────────────────

#[component]
fn ScanTable() -> Element {
    let state = use_context::<AppState>();
    if state.scans.read().is_empty() {
        return rsx! {
            div { class: "empty-state",
                p { "No scans yet." }
                p { style: "margin-top:6px;font-size:12px;", "Scan the QR code with the phone app to connect." }
            }
        };
    }
    let show_hidden = (state.show_hidden)();
    let entries: Vec<(usize, ScanEntry)> = state
        .scans
        .read()
        .iter()
        .enumerate()
        .filter(|(_, e)| show_hidden || !e.hidden)
        .map(|(i, e)| (i, e.clone()))
        .collect();
    rsx! {
        table {
            thead { tr {
                th { class: "check-col", "" }
                th { "#" } th { "Kind" } th { "Code" } th { "Extracted" } th { "Shop" } th { "Img" } th { "Time" }
            }}
            tbody {
                for (i, entry) in entries.into_iter().rev() {
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
        tr { class: "{row_cls}", style: "cursor:pointer;",
            onclick: move |_| {
                state.selected_scan.set(Some(i));
                state.pin_input.set(String::new());
                let shops = &state.scans.read()[i].detected_shops;
                if shops.len() == 1 {
                    state.selected_shop.set(Some(shops[0].to_string()));
                } else {
                    state.selected_shop.set(None);
                }
            },
            td { class: "check-col",
                input {
                    r#type: "checkbox",
                    checked: is_hidden,
                    onclick: move |e| {
                        e.stop_propagation();
                        state.scans.write()[i].hidden = !is_hidden;
                    },
                }
            }
            td { "{i + 1}" }
            td { "{entry.kind}" }
            td { style: "font-family:inherit;max-width:220px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;",
                "{entry.code}"
            }
            td { style: "font-family:monospace;max-width:180px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;",
                if let Some(extracted) = &entry.extracted_card {
                    rsx! {
                        span { style: "color:green;font-weight:bold;cursor:pointer;",
                            onclick: move |e| {
                                e.stop_propagation();
                                copy_to_clipboard(extracted);
                            },
                            "{extracted} 📋"
                        }
                    }
                } else {
                    rsx! {
                        span { style: "color:#999;", "--" }
                    }
                }
            }
            td { "{shop_str}" }
            td {
                if has_image {
                    img { class: "thumb", src: "{src}",
                        onclick: move |e| {
                            e.stop_propagation();
                            state.selected_image.set(Some(i));
                            state.image_rotation.set(0);
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
                match barcode_proto::recv_scan(&mut send, &mut recv).await {
                    Ok(result) => {
                        let image_b64 = if result.image_jpeg.is_empty() {
                            String::new()
                        } else {
                            BASE64.encode(&result.image_jpeg)
                        };
                        let detected_shops = detect_shops(&result.code);
                        let entry = ScanEntry {
                            kind: result.kind.as_str().to_string(),
                            code: result.code.clone(),
                            extracted_card: result.extracted_card,
                            image_b64,
                            timestamp: format_timestamp(),
                            detected_shops,
                            hidden: false,
                        };
                        let display_code = entry.extracted_card.as_ref().unwrap_or(&entry.code);
                        info!("Scan: {} - {} (extracted: {})", entry.kind, entry.code, display_code);
                        let _ = tx.send(IrohEvent::Scan(entry));
                    }
                    Err(e) => {
                        tracing::warn!("recv error: {e:#}");
                        break;
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
