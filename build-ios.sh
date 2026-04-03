#!/usr/bin/env bash
# build-ios.sh — Build the Rust C-FFI staticlib for iOS and package it as an XCFramework.
#
# Requirements (macOS only):
#   - Rust with iOS targets: rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
#   - Xcode command-line tools (xcodebuild, lipo, xcrun)
#
# Output:
#   dist/BarcodeScanner.xcframework
#
# Usage:
#   ./build-ios.sh [--debug]
#
# By default builds in release mode. Pass --debug for faster incremental builds.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CRATE_DIR="$SCRIPT_DIR/ios/rust"
HEADER_DIR="$CRATE_DIR/src/include"
DIST_DIR="$SCRIPT_DIR/dist"
XCFW_DIR="$DIST_DIR/BarcodeScanner.xcframework"
TMP_DIR="$DIST_DIR/.xcfw_tmp"

# Parse arguments
PROFILE="release"
CARGO_FLAGS="--release"
if [[ "${1:-}" == "--debug" ]]; then
    PROFILE="debug"
    CARGO_FLAGS=""
fi

LIB_NAME="libbarcode_scanner_ios.a"

# ios/rust is excluded from the root workspace, so Cargo uses ios/rust/target/ (not barcodeapp/target/)
TARGET_DIR="$CRATE_DIR/target"

echo "==> Building Rust iOS targets (profile: $PROFILE)"
echo ""

# ---------------------------------------------------------------------------
# 1. Build for physical device (ARM64)
# ---------------------------------------------------------------------------
echo "--> aarch64-apple-ios (device)"
cargo build \
    --manifest-path "$CRATE_DIR/Cargo.toml" \
    --target aarch64-apple-ios \
    $CARGO_FLAGS

# ---------------------------------------------------------------------------
# 2. Build for simulator (ARM64 — Apple Silicon Macs)
# ---------------------------------------------------------------------------
echo "--> aarch64-apple-ios-sim (simulator, Apple Silicon)"
cargo build \
    --manifest-path "$CRATE_DIR/Cargo.toml" \
    --target aarch64-apple-ios-sim \
    $CARGO_FLAGS

# ---------------------------------------------------------------------------
# 3. Build for simulator (x86_64 — Intel Macs)
# ---------------------------------------------------------------------------
echo "--> x86_64-apple-ios (simulator, Intel)"
cargo build \
    --manifest-path "$CRATE_DIR/Cargo.toml" \
    --target x86_64-apple-ios \
    $CARGO_FLAGS

echo ""
echo "==> Creating XCFramework..."
rm -rf "$TMP_DIR" "$XCFW_DIR"
mkdir -p "$TMP_DIR"

# Paths to compiled archives
DEVICE_LIB="$TARGET_DIR/aarch64-apple-ios/$PROFILE/$LIB_NAME"
SIM_ARM64_LIB="$TARGET_DIR/aarch64-apple-ios-sim/$PROFILE/$LIB_NAME"
SIM_X86_LIB="$TARGET_DIR/x86_64-apple-ios/$PROFILE/$LIB_NAME"

# ---------------------------------------------------------------------------
# 4. Lipo simulator slices into a fat binary
# ---------------------------------------------------------------------------
echo "--> lipo simulator slices"
SIM_FAT_LIB="$TMP_DIR/$LIB_NAME"
lipo -create "$SIM_ARM64_LIB" "$SIM_X86_LIB" -output "$SIM_FAT_LIB"

# ---------------------------------------------------------------------------
# 5. Wrap each slice in a .framework directory
#    XCFramework requires framework bundles, not bare .a files.
# ---------------------------------------------------------------------------
make_framework() {
    local DEST="$1"         # directory to create .framework in
    local LIB_PATH="$2"     # path to .a file
    local FW_DIR="$DEST/BarcodeScanner.framework"
    mkdir -p "$FW_DIR/Headers"
    cp "$LIB_PATH" "$FW_DIR/BarcodeScanner"
    cp "$HEADER_DIR"/*.h "$FW_DIR/Headers/"
    # Minimal Info.plist for a static framework
    cat > "$FW_DIR/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>com.kyosk.barcodescanner.rust</string>
    <key>CFBundleName</key>
    <string>BarcodeScanner</string>
    <key>CFBundlePackageType</key>
    <string>FMWK</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>MinimumOSVersion</key>
    <string>16.0</string>
</dict>
</plist>
PLIST
}

DEVICE_FW_DIR="$TMP_DIR/device"
SIM_FW_DIR="$TMP_DIR/simulator"

make_framework "$DEVICE_FW_DIR" "$DEVICE_LIB"
make_framework "$SIM_FW_DIR" "$SIM_FAT_LIB"

# ---------------------------------------------------------------------------
# 6. Assemble XCFramework
# ---------------------------------------------------------------------------
mkdir -p "$DIST_DIR"
xcodebuild -create-xcframework \
    -framework "$DEVICE_FW_DIR/BarcodeScanner.framework" \
    -framework "$SIM_FW_DIR/BarcodeScanner.framework" \
    -output "$XCFW_DIR"

rm -rf "$TMP_DIR"

echo ""
echo "==> Done! XCFramework at: $XCFW_DIR"
echo ""
ls -lh "$XCFW_DIR"
