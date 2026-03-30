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
    /// Number of digit counts that map to this shop.
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

/// Detect shop from digit count. Returns shop name(s).
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
    image_b64: String,
    timestamp: String,
    detected_shops: Vec<&'static str>,
}

enum IrohEvent {
    Status(String),
    Ticket { ticket_str: String, qr_svg: String },
    Scan(ScanEntry),
}

// ── Clipboard helper ─────────────────────────────────────────────────

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

// ── CSS ──────────────────────────────────────────────────────────────

const CSS: &str = r#"
:root { color-scheme: dark; }
* { margin: 0; padding: 0; box-sizing: border-box; }
body {
    background: #1a1a2e;
    color: #e0e0e0;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    min-height: 100vh;
}
.header {
    display: flex; align-items: center; justify-content: space-between;
    padding: 16px 24px; background: #16213e; border-bottom: 1px solid #0f3460;
}
.header h1 { font-size: 20px; color: #e94560; }
.btn {
    padding: 8px 16px; border: 1px solid #0f3460; background: #0f3460;
    color: #e0e0e0; border-radius: 6px; cursor: pointer; font-size: 14px;
    white-space: nowrap;
}
.btn:hover { background: #1a4a8a; }
.btn-sm { padding: 4px 10px; font-size: 12px; }
.btn-green { background: #1b5e20; border-color: #2e7d32; }
.btn-green:hover { background: #2e7d32; }
.btn-orange { background: #e65100; border-color: #ef6c00; }
.btn-orange:hover { background: #ef6c00; }
.btn-copy { background: #37474f; border-color: #546e7a; font-size: 12px; padding: 4px 8px; }
.btn-copy:hover { background: #546e7a; }
.btn-selected { background: #2e7d32 !important; border-color: #4caf50 !important; }
.overlay {
    position: fixed; top: 0; left: 0; right: 0; bottom: 0;
    background: rgba(0,0,0,0.80); display: flex; flex-direction: column;
    align-items: center; justify-content: center; z-index: 100;
}
.overlay-content {
    background: #16213e; padding: 32px; border-radius: 12px;
    text-align: center; max-width: 90vw; max-height: 90vh; overflow: auto;
}
.qr-svg { margin: 16px auto; display: block; }
.qr-svg svg { width: 280px; height: 280px; }
.ticket-str {
    font-family: monospace; font-size: 11px; word-break: break-all;
    max-width: 400px; margin: 12px auto; padding: 8px;
    background: #1a1a2e; border-radius: 6px; user-select: all; color: #aaa;
}
.content { padding: 24px; }
.empty-state { text-align: center; padding: 64px; color: #666; font-size: 16px; }

/* Detail panel for selected scan */
.detail-panel {
    background: #16213e; border: 1px solid #0f3460; border-radius: 8px;
    padding: 20px; margin-bottom: 20px;
}
.detail-panel h3 { margin-bottom: 12px; color: #e94560; font-size: 16px; }
.detail-row {
    display: flex; align-items: center; gap: 10px; margin-bottom: 10px;
}
.detail-row label { font-size: 13px; color: #888; min-width: 80px; }
.detail-row input {
    flex: 1; padding: 6px 10px; background: #1a1a2e; border: 1px solid #333;
    border-radius: 4px; color: #e0e0e0; font-family: monospace; font-size: 14px;
}
.detail-row input:focus { outline: none; border-color: #0f3460; }
.shop-buttons { display: flex; gap: 8px; flex-wrap: wrap; margin-top: 8px; }
.shop-detect {
    font-size: 12px; padding: 4px 8px; border-radius: 4px; margin-bottom: 8px;
}
.shop-detect.found { background: #1b5e20; color: #a5d6a7; }
.shop-detect.ambiguous { background: #e65100; color: #ffcc80; }
.shop-detect.none { background: #37474f; color: #90a4ae; }
.action-row { display: flex; gap: 8px; margin-top: 12px; align-items: center; }

table { width: 100%; border-collapse: collapse; margin-top: 16px; }
th {
    background: #16213e; padding: 10px 12px; text-align: left;
    font-size: 13px; color: #888; border-bottom: 2px solid #0f3460;
}
td {
    padding: 10px 12px; border-bottom: 1px solid #2a2a4a;
    font-size: 14px; vertical-align: middle;
}
tr:hover { background: #1e1e3a; }
tr.selected-row { background: #0f3460; }
.thumb {
    width: 64px; height: 64px; object-fit: cover; border-radius: 4px;
    cursor: pointer; border: 1px solid #333;
}
.thumb:hover { border-color: #e94560; }
.modal-image { max-width: 80vw; max-height: 80vh; border-radius: 8px; }
.status { font-size: 12px; color: #666; padding: 8px 24px; }
.copied-toast {
    color: #4caf50; font-size: 12px; margin-left: 8px;
    animation: fadeout 1.5s forwards;
}
@keyframes fadeout { 0% { opacity: 1; } 100% { opacity: 0; } }
"#;

// ── App component ────────────────────────────────────────────────────

#[component]
fn App() -> Element {
    let mut scans: Signal<Vec<ScanEntry>> = use_signal(Vec::new);
    let mut ticket_str: Signal<String> = use_signal(String::new);
    let mut qr_svg: Signal<String> = use_signal(String::new);
    let mut show_qr = use_signal(|| true);
    let mut selected_image: Signal<Option<usize>> = use_signal(|| None);
    let mut status_msg: Signal<String> = use_signal(|| "Starting...".to_string());
    let mut selected_scan: Signal<Option<usize>> = use_signal(|| None);
    let mut pin_input: Signal<String> = use_signal(String::new);
    let mut selected_shop: Signal<Option<String>> = use_signal(|| None);
    let mut copy_feedback: Signal<Option<&'static str>> = use_signal(|| None);

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

    // Drain events from iroh into signals
    use_future(move || async move {
        let mut rx = match rx_holder.with_mut(|r| r.take()) {
            Some(rx) => rx,
            None => return,
        };
        while let Some(event) = rx.recv().await {
            match event {
                IrohEvent::Status(msg) => status_msg.set(msg),
                IrohEvent::Ticket {
                    ticket_str: ts,
                    qr_svg: qr,
                } => {
                    ticket_str.set(ts);
                    qr_svg.set(qr);
                }
                IrohEvent::Scan(entry) => {
                    // Auto-select newly arrived scan
                    let idx = scans.read().len();
                    scans.write().push(entry.clone());
                    selected_scan.set(Some(idx));
                    pin_input.set(String::new());
                    // Auto-detect shop
                    let detected = &entry.detected_shops;
                    if detected.len() == 1 {
                        selected_shop.set(Some(detected[0].to_string()));
                    } else {
                        selected_shop.set(None);
                    }
                }
            }
        }
    });

    let scan_count = scans.read().len();

    rsx! {
        style { {CSS} }

        // Header
        div {
            class: "header",
            h1 { "Barcode Receiver" }
            div {
                style: "display: flex; align-items: center; gap: 12px;",
                span {
                    style: "font-size: 13px; color: #888;",
                    "{scan_count} scan(s)"
                }
                button {
                    class: "btn",
                    onclick: move |_| show_qr.set(!show_qr()),
                    if show_qr() { "Hide QR" } else { "Show QR" }
                }
            }
        }

        div { class: "status", "{status_msg}" }

        // QR overlay
        if show_qr() && !qr_svg.read().is_empty() {
            div {
                class: "overlay",
                onclick: move |_| show_qr.set(false),
                div {
                    class: "overlay-content",
                    onclick: move |e| e.stop_propagation(),
                    h2 { style: "margin-bottom: 8px; font-size: 18px;", "Scan to connect" }
                    div { class: "qr-svg", dangerous_inner_html: "{qr_svg}" }
                    p {
                        style: "font-size: 12px; color: #888; margin-bottom: 4px;",
                        "Endpoint ticket (tap to select):"
                    }
                    div { class: "ticket-str", "{ticket_str}" }
                    button {
                        class: "btn",
                        onclick: move |_| show_qr.set(false),
                        "Close"
                    }
                }
            }
        }

        // Image modal
        if let Some(idx) = selected_image() {
            {
                let scans_read = scans.read();
                if let Some(entry) = scans_read.get(idx) {
                    let src = format!("data:image/jpeg;base64,{}", entry.image_b64);
                    rsx! {
                        div {
                            class: "overlay",
                            onclick: move |_| selected_image.set(None),
                            div {
                                class: "overlay-content",
                                onclick: move |e| e.stop_propagation(),
                                img { class: "modal-image", src: "{src}" }
                                p {
                                    style: "margin-top: 12px; font-size: 14px;",
                                    "{entry.kind}: {entry.code}"
                                }
                                button {
                                    class: "btn",
                                    style: "margin-top: 12px;",
                                    onclick: move |_| selected_image.set(None),
                                    "Close"
                                }
                            }
                        }
                    }
                } else {
                    rsx! {}
                }
            }
        }

        // Main content
        div {
            class: "content",

            // Detail panel (shown when a scan is selected)
            if let Some(sel_idx) = selected_scan() {
                {
                    let scans_read = scans.read();
                    if let Some(entry) = scans_read.get(sel_idx) {
                        let code_val = entry.code.clone();
                        let detected = entry.detected_shops.clone();
                        let det_len = detected.len();
                        let code_for_copy = code_val.clone();
                        let pin_for_copy = pin_input.read().clone();
                        rsx! {
                            div {
                                class: "detail-panel",
                                h3 { "Scan #{sel_idx + 1} — {entry.kind}" }

                                // Shop detection status
                                if det_len == 1 {
                                    div {
                                        class: "shop-detect found",
                                        "Detected: {detected[0]}"
                                    }
                                } else if det_len > 1 {
                                    div {
                                        class: "shop-detect ambiguous",
                                        "Ambiguous: could be {detected:?} — please choose"
                                    }
                                } else {
                                    div {
                                        class: "shop-detect none",
                                        "No shop auto-detected"
                                    }
                                }

                                // Card number row
                                div {
                                    class: "detail-row",
                                    label { "Card #:" }
                                    input {
                                        r#type: "text",
                                        value: "{code_val}",
                                        readonly: true,
                                    }
                                    button {
                                        class: "btn btn-copy",
                                        onclick: move |_| {
                                            copy_to_clipboard(&code_for_copy);
                                            copy_feedback.set(Some("Card copied!"));
                                            // Clear feedback after delay via spawn
                                            spawn(async move {
                                                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                                                copy_feedback.set(None);
                                            });
                                        },
                                        "Copy Card"
                                    }
                                }

                                // PIN row
                                div {
                                    class: "detail-row",
                                    label { "PIN:" }
                                    input {
                                        r#type: "text",
                                        value: "{pin_input}",
                                        maxlength: "6",
                                        placeholder: "Enter PIN",
                                        oninput: move |e: Event<FormData>| {
                                            pin_input.set(e.value());
                                        },
                                    }
                                    button {
                                        class: "btn btn-copy",
                                        onclick: move |_| {
                                            let p = pin_for_copy.clone();
                                            if !p.is_empty() {
                                                copy_to_clipboard(&p);
                                                copy_feedback.set(Some("PIN copied!"));
                                                spawn(async move {
                                                    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                                                    copy_feedback.set(None);
                                                });
                                            }
                                        },
                                        "Copy PIN"
                                    }
                                }

                                // Copy feedback toast
                                if let Some(msg) = copy_feedback() {
                                    span { class: "copied-toast", "{msg}" }
                                }

                                // Shop buttons
                                div {
                                    style: "margin-top: 12px;",
                                    label { style: "font-size: 13px; color: #888;", "Shop:" }
                                    div {
                                        class: "shop-buttons",
                                        for shop in SHOPS.iter() {
                                            {
                                                let shop_name = shop.name.to_string();
                                                let shop_name2 = shop_name.clone();
                                                let is_selected = selected_shop.read().as_deref() == Some(shop.name);
                                                let cls = if is_selected {
                                                    "btn btn-sm btn-selected"
                                                } else if detected.contains(&shop.name) {
                                                    "btn btn-sm btn-green"
                                                } else {
                                                    "btn btn-sm"
                                                };
                                                rsx! {
                                                    button {
                                                        class: "{cls}",
                                                        onclick: move |_| {
                                                            if selected_shop.read().as_deref() == Some(&shop_name) {
                                                                selected_shop.set(None);
                                                            } else {
                                                                selected_shop.set(Some(shop_name.clone()));
                                                            }
                                                        },
                                                        "{shop_name2}"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                // Action buttons: Open in Browser
                                div {
                                    class: "action-row",
                                    button {
                                        class: "btn btn-green",
                                        disabled: selected_shop.read().is_none(),
                                        onclick: move |_| {
                                            if let Some(name) = selected_shop.read().as_deref()
                                                && let Some(url) = shop_url(name)
                                                    && let Err(e) = opener::open(url) {
                                                        tracing::warn!("failed to open browser: {e}");
                                                    }
                                        },
                                        "Open in Browser"
                                    }
                                    if let Some(name) = selected_shop.read().as_deref() {
                                        span {
                                            style: "font-size: 12px; color: #888;",
                                            {
                                                let url = shop_url(name).unwrap_or("");
                                                url.to_string()
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        rsx! {}
                    }
                }
            }

            // Scan table
            if scans.read().is_empty() {
                div {
                    class: "empty-state",
                    p { "No scans yet." }
                    p {
                        style: "margin-top: 8px; font-size: 13px;",
                        "Point the scanner app at the QR code to connect."
                    }
                }
            } else {
                table {
                    thead {
                        tr {
                            th { "#" }
                            th { "Kind" }
                            th { "Code" }
                            th { "Shop" }
                            th { "Image" }
                            th { "Time" }
                        }
                    }
                    tbody {
                        for (i, entry) in scans.read().iter().enumerate().rev() {
                            {
                                let src = format!("data:image/jpeg;base64,{}", entry.image_b64);
                                let has_image = !entry.image_b64.is_empty();
                                let is_selected = selected_scan() == Some(i);
                                let row_cls = if is_selected { "selected-row" } else { "" };
                                let detected_str = if entry.detected_shops.is_empty() {
                                    "—".to_string()
                                } else {
                                    entry.detected_shops.join(" / ")
                                };
                                rsx! {
                                    tr {
                                        class: "{row_cls}",
                                        style: "cursor: pointer;",
                                        onclick: move |_| {
                                            selected_scan.set(Some(i));
                                            pin_input.set(String::new());
                                            let shops = &scans.read()[i].detected_shops;
                                            if shops.len() == 1 {
                                                selected_shop.set(Some(shops[0].to_string()));
                                            } else {
                                                selected_shop.set(None);
                                            }
                                        },
                                        td { "{i + 1}" }
                                        td { "{entry.kind}" }
                                        td {
                                            style: "font-family: monospace; max-width: 250px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;",
                                            "{entry.code}"
                                        }
                                        td { "{detected_str}" }
                                        td {
                                            if has_image {
                                                img {
                                                    class: "thumb",
                                                    src: "{src}",
                                                    onclick: move |e| {
                                                        e.stop_propagation();
                                                        selected_image.set(Some(i));
                                                    },
                                                }
                                            } else {
                                                span { style: "color: #555;", "N/A" }
                                            }
                                        }
                                        td {
                                            style: "font-size: 12px; color: #888;",
                                            "{entry.timestamp}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
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
    let _ = tx.send(IrohEvent::Status(
        "Waiting for endpoint to come online...".to_string(),
    ));

    ep.online().await;

    let addr = ep.addr();
    info!("Endpoint address: {:?}", addr);

    let ticket = EndpointTicket::new(addr);
    let ts = ticket.serialize();
    info!("Ticket: {}", ts);

    let code = QrCode::new(ts.as_bytes()).map_err(|e| anyhow::anyhow!("QR encode: {e}"))?;
    let svg_string = code.render::<svg::Color>().min_dimensions(280, 280).build();

    let _ = tx.send(IrohEvent::Ticket {
        ticket_str: ts,
        qr_svg: svg_string,
    });
    let _ = tx.send(IrohEvent::Status(format!(
        "Online - listening as {}",
        ep.id()
    )));

    loop {
        let incoming = match ep.accept().await {
            Some(incoming) => incoming,
            None => {
                info!("Endpoint closed");
                let _ = tx.send(IrohEvent::Status("Endpoint closed".to_string()));
                break;
            }
        };

        let conn = match incoming.accept() {
            Ok(accepting) => match accepting.await {
                Ok(conn) => conn,
                Err(e) => {
                    tracing::warn!("Connection failed: {e:#}");
                    continue;
                }
            },
            Err(e) => {
                tracing::warn!("Accept failed: {e:#}");
                continue;
            }
        };

        info!("New connection from {:?}", conn.remote_id());

        let tx = tx.clone();
        tokio::spawn(async move {
            loop {
                let (mut send, mut recv) = match conn.accept_bi().await {
                    Ok(pair) => pair,
                    Err(e) => {
                        info!("Connection stream ended: {e:#}");
                        break;
                    }
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
                            code: result.code,
                            image_b64,
                            timestamp: format_timestamp(),
                            detected_shops,
                        };
                        info!("Scan received: {} - {}", entry.kind, entry.code);
                        let _ = tx.send(IrohEvent::Scan(entry));
                    }
                    Err(e) => {
                        tracing::warn!("recv_scan error: {e:#}");
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
    format!("{h:02}:{m:02}:{s:02} UTC")
}
