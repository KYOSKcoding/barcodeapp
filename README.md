# Barcode Scanner

Scan barcodes and QR codes on an Android phone, send them instantly to a Linux desktop app over peer-to-peer networking.

## Overview

Two apps talk to each other over [iroh](https://iroh.computer/), a QUIC-based p2p library. The phone scans codes with its camera and transmits the code text plus a low-res image to the desktop. The desktop shows a QR code to pair, then displays incoming scans in a table with shop detection, copy buttons, and a browser shortcut for gift card balance checks. No server needed — the devices connect directly via relay or local network (mDNS).

## Usage

Download both apps from the [latest release](https://github.com/Frando/barcodeapp/releases/tag/latest):

- **barcode-receiver-x86_64.AppImage** — desktop receiver (Linux x86_64)
- **barcode-scanner.apk** — phone scanner (Android arm64)

### Install

Make the AppImage executable and run it:

```sh
chmod +x barcode-receiver-x86_64.AppImage
./barcode-receiver-x86_64.AppImage
```

Transfer the APK to your phone and install it. The easiest way is `adb install`:

```sh
adb install barcode-scanner.apk
```

You can also copy the APK to your phone via USB, a file sharing app, or by opening the download link directly on the phone's browser. Android will ask you to allow installation from unknown sources — confirm and install.

### Scan

1. Launch the receiver on your desktop. A QR code appears on screen.
2. Open the scanner app on your phone, tap **Start**, and scan the QR code.
3. Once connected, tap **Scan** to read barcodes or QR codes with the camera.
4. Each scan shows up on the desktop in real time. Use the shop buttons, copy, and browser features from there.
5. Tap **Disconnect** on the phone to return to the start screen and re-pair.

## Building

### Prerequisites

- Rust stable (2024 edition): https://rustup.rs
- Linux system libraries for the desktop app: `libgtk-3-dev`, `libwebkit2gtk-4.1-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, `libxdo-dev`
- For the Android app: Android SDK with `ANDROID_HOME` set, NDK 28.x, plus `cargo install cargo-ndk cargo-make` and `rustup target add aarch64-linux-android`

### Desktop receiver

```sh
cargo run -p receiver            # debug
cargo build -p receiver --release  # release binary in target/release/receiver
```

### Android scanner

```sh
cd android
cargo make run-on-device          # build, install on connected device, stream logs
cargo make run-on-device-release  # same with release build
cargo make logcat                 # stream filtered logs only
```

### CI

`.github/workflows/release.yml` runs on every push to `main`. It creates a `latest` tag and GitHub release first, then builds the APK and AppImage in parallel. Each artifact is uploaded as soon as its build finishes, so you don't have to wait for both.

## Architecture

```
+-------------------+                          +--------------------+
|  Android Phone    |      iroh (QUIC p2p)     |  Desktop (Linux)   |
|                   |  <---------------------> |                    |
|  Kotlin UI        |    ALPN: barcodescan/0   |  Dioxus UI         |
|  + Rust JNI lib   |                          |  + iroh endpoint   |
|  + ZXing scanner  |    bidi streams over     |  + QR code display |
|                   |    direct / relayed conn |  + scan table      |
+-------------------+                          +--------------------+
```

### `barcode-proto/` — shared protocol crate

Defines the ALPN identifier (`barcodescan/0`), wire format, and async `send_scan` / `recv_scan` functions used by both sides. Each scanned code travels over a single bidirectional QUIC stream: the scanner sends kind (barcode/QR), code string, and JPEG image; the receiver replies with a one-byte ACK; both sides finish.

### `android/` — scanner app

Kotlin activity with a state machine (idle, connecting, scanning, sending) and two zxing-embedded scan launchers — one for the connection ticket QR, one for barcodes. The iroh networking runs in Rust, accessed through four JNI functions: `connect`, `sendScan`, `isConnected`, `disconnect`. A disconnect button lets you return to the start screen and re-pair.

### `receiver/` — desktop app

Dioxus 0.7 desktop app. On launch it creates an iroh endpoint with N0 preset and mDNS, generates an `EndpointTicket`, and shows it as a QR code (compact ID-only by default, with a toggle for the full ticket including addresses). Incoming scans populate a table. The detail panel auto-detects the shop (REWE, DM, ALDI, LIDL) from digit count heuristics and offers copy-to-clipboard for card number and PIN, plus an "Open in Browser" button for the shop's balance-check page. An image viewer with rotation is available for scans that include a photo. A checkbox on each row hides processed entries; a header toggle reveals them again.

### Tech stack

iroh 0.97 (p2p), Dioxus 0.7 (desktop UI), Kotlin + CameraX (Android UI), zxing-embedded (barcode scanning), JNI via `jni` 0.21 (Rust-Android bridge), Tokio (async runtime), `qrcode` (QR generation), `arboard` (clipboard), `opener` (browser launch).
