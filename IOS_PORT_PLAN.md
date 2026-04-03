# iOS Port Plan: Barcode Scanner

## Architecture Overview

The existing app is a **Rust + Kotlin** Android scanner that:
1. Uses `barcode-proto` (pure Rust) for the wire protocol over iroh/QUIC
2. Uses `android/rust/` as a JNI cdylib bridge exposing 4 functions to Kotlin
3. Uses `zxing-embedded` for camera/barcode scanning
4. Calls iroh for encrypted QUIC P2P transport to a desktop receiver

For iOS, the plan is:

```
┌─────────────────────────────────────────────────────────────┐
│                     shared/ (KMP module)                     │
│   commonMain: ShopInfo, extractCardNumbers, detectShops,     │
│               BarcodeState enum                              │
└────────────────────────┬────────────────────────────────────┘
                         │
         ┌───────────────┴───────────────┐
         │                               │
┌────────▼────────┐             ┌────────▼────────┐
│  Android App    │             │    iOS App       │
│  (existing      │             │  (new SwiftUI)   │
│   Kotlin +      │             │                  │
│   JNI bridge)   │             │  ← shared.xcfw   │
└────────┬────────┘             └────────┬─────────┘
         │                               │
┌────────▼────────┐             ┌────────▼─────────┐
│ android/rust/   │             │ ios/rust/         │
│ (JNI cdylib)    │             │ (C-FFI staticlib) │
│ barcode-proto   │             │ barcode-proto     │
└─────────────────┘             └──────────────────┘
```

---

## Step 1: Rust iOS Targets

### Targets to add
| Target | Use |
|--------|-----|
| `aarch64-apple-ios` | Physical device (iPhone/iPad ARM64) |
| `aarch64-apple-ios-sim` | Simulator on Apple Silicon Mac |
| `x86_64-apple-ios` | Simulator on Intel Mac |

### New crate: `ios/rust/`
- Crate type: `staticlib` (not cdylib — iOS requires static linking)
- Exposes C-FFI functions (not JNI) with `#[no_mangle] pub extern "C"`
- Same 4 functions as JNI bridge: connect, send_scan, is_connected, disconnect
- Shares `barcode-proto` from workspace

### C-FFI function signatures
```c
int64_t barcode_scanner_connect(const char* ticket);
bool    barcode_scanner_send_scan(int64_t handle, int32_t kind,
                                  const char* code,
                                  const uint8_t* image_jpeg, size_t image_len);
bool    barcode_scanner_is_connected(int64_t handle);
void    barcode_scanner_disconnect(int64_t handle);
```

### XCFramework build
`build-ios.sh` script:
1. `rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios`
2. `cargo build --target aarch64-apple-ios --release` (device)
3. `cargo build --target aarch64-apple-ios-sim --release` (simulator arm64)
4. `cargo build --target x86_64-apple-ios --release` (simulator x86)
5. `lipo` arm64-sim + x86 sim into fat simulator binary
6. Wrap each into a `.framework` directory with headers
7. `xcodebuild -create-xcframework` → `BarcodeScanner.xcframework`

### Workspace changes
- Add `ios/rust` to root `Cargo.toml` workspace members (non-Android only)
- The `exclude = ["android/rust"]` pattern remains; `ios/rust` compiles conditionally

---

## Step 2: Kotlin Multiplatform Setup

### New module: `shared/`
Lives alongside `android/` in the repo root. Gradle settings updated to include it.

**Structure:**
```
shared/
├── build.gradle.kts          ← KMP plugin, targets: android, iosArm64, iosSimulatorArm64, iosX64
└── src/
    └── commonMain/kotlin/com/example/barcodescanner/
        ├── ShopInfo.kt       ← ShopInfo data class + SHOPS list
        ├── CardExtraction.kt ← extractCardNumbers(), detectShops()
        └── BarcodeState.kt   ← State enum
```

No `androidMain` or `iosMain` needed for this pure-logic module (no expect/actual).

### Android app changes
- `android/settings.gradle.kts`: include `":shared"` 
- `android/app/build.gradle.kts`: add `implementation(project(":shared"))`
- Remove duplicate shop/extraction logic from `MainActivity.kt`, import from shared

### iOS consumption
KMP Gradle plugin produces `shared.framework` for iOS targets via:
```
./gradlew :shared:assembleSharedXCFramework
```
The output `shared.xcframework` is linked into the iOS Xcode project alongside `BarcodeScanner.xcframework` (Rust C-FFI).

---

## Step 3: C-FFI Bridge for iOS

### ios/rust/src/lib.rs
Mirrors `android/rust/src/lib.rs` but:
- No `jni` dependency
- No `ndk-context`
- No `logcat.rs` (use `tracing-subscriber` with stdout/stderr writer)
- Functions use `extern "C"` with raw pointer arguments
- Same `SessionHandle`, same global Tokio runtime, same `do_connect`/`do_send` async logic

### Swift wrapper: `ios/BarcodeScanner/IrohBridge.swift`
```swift
import Foundation

// Wraps the C-FFI Rust library as an async Swift class
actor IrohBridge {
    private var handle: Int64 = 0

    func connect(ticket: String) async -> Bool { ... }
    func sendScan(kind: Int32, code: String, imageJpeg: Data) async -> Bool { ... }
    var isConnected: Bool { barcode_scanner_is_connected(handle) }
    func disconnect() { ... }
}
```

### Bridging header
`ios/BarcodeScanner/BarcodeScanner-Bridging-Header.h`:
```c
#include "../../ios/rust/src/include/barcode_scanner.h"
```

---

## Step 4: iOS UI (SwiftUI)

### State machine (mirrors Android)
```swift
enum ScannerState {
    case idle
    case scanningTicket
    case connecting
    case ready
    case scanningCode
    case scanned(code: String, kind: Int32, image: Data?)
    case sending
}
```

### Views
- `ContentView.swift` — root view with state-driven UI
- `BarcodeScanner.swift` — AVFoundation camera scanner wrapped in `UIViewControllerRepresentable`

### Permissions
`Info.plist`:
- `NSCameraUsageDescription`
- `NSLocalNetworkUsageDescription` (for iroh mDNS)

### Barcode scanning
- `AVCaptureSession` with `AVMetadataObjectTypeQRCode`, `AVMetadataObjectTypeEAN13Code`, etc.
- Image capture via `AVCapturePhotoOutput` → JPEG → `Data`
- No zxing needed

---

## Step 5: Xcode Project Setup

### Approach: xcodegen
Use [xcodegen](https://github.com/yonaskolb/XcodeGen) with `ios/project.yml` to generate
`ios/BarcodeScanner.xcodeproj` reproducibly (no binary blob in git).

### project.yml key settings
- Platform: iOS 16.0+
- Linked frameworks: `BarcodeScanner.xcframework` (Rust), `shared.xcframework` (KMP)
- Bridging header for C-FFI
- Build phases: pre-action to run `build-ios.sh` and `./gradlew assembleSharedXCFramework`

### Code signing
- For CI: `CODE_SIGNING_ALLOWED=NO` or ad-hoc signing for simulator
- For device/distribution: App Store Connect + provisioning profile
- Team ID from `APPLE_TEAM_ID` secret

### Entitlements
```xml
<key>com.apple.developer.networking.multicast</key>
<true/>
```
Required for iroh mDNS to function on iOS (local network discovery).
**Note:** Apple must grant the multicast entitlement; alternative is to skip mDNS and use full ticket URLs.

---

## Step 6: Updated GitHub Actions

### New file: `.github/workflows/build-ios.yml`
```yaml
on:
  push:
    branches: [main]

jobs:
  build-ios:
    runs-on: macos-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install Rust iOS targets
        run: rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
      - name: Set up JDK 17
        uses: actions/setup-java@v4
        with: { java-version: '17', distribution: 'temurin' }
      - name: Build Rust XCFramework
        run: ./build-ios.sh
      - name: Build KMP shared framework
        run: cd android && ./gradlew :shared:assembleSharedXCFramework
      - name: Install xcodegen
        run: brew install xcodegen
      - name: Generate Xcode project
        run: cd ios && xcodegen
      - name: Build for simulator (smoke test)
        run: |
          xcodebuild -project ios/BarcodeScanner.xcodeproj \
            -scheme BarcodeScanner \
            -destination 'platform=iOS Simulator,name=iPhone 16' \
            -configuration Debug \
            CODE_SIGNING_ALLOWED=NO \
            build
      - name: Archive for distribution (optional)
        if: false  # Enable when signing secrets are configured
        env:
          APPLE_TEAM_ID: ${{ secrets.APPLE_TEAM_ID }}
          APPLE_CERTIFICATE: ${{ secrets.APPLE_CERTIFICATE }}
          APPLE_PROVISIONING_PROFILE: ${{ secrets.APPLE_PROVISIONING_PROFILE }}
        run: |
          # Import certificate and provisioning profile, then:
          xcodebuild archive ...
          xcodebuild -exportArchive ...
```

### Secrets required (for distribution)
| Secret | Description |
|--------|-------------|
| `APPLE_TEAM_ID` | 10-character Apple Developer Team ID |
| `APPLE_CERTIFICATE` | Base64-encoded `.p12` distribution certificate |
| `APPLE_CERTIFICATE_PASSWORD` | Password for the `.p12` |
| `APPLE_PROVISIONING_PROFILE` | Base64-encoded `.mobileprovision` |

---

## Step 7: Smoke Test

Build target: **iOS Simulator** (no signing required)

```bash
# After all steps complete:
cd ios
xcodegen
xcodebuild \
  -project BarcodeScanner.xcodeproj \
  -scheme BarcodeScanner \
  -destination 'platform=iOS Simulator,name=iPhone 16' \
  CODE_SIGNING_ALLOWED=NO \
  build
```

Success = no compilation errors. UI can be tested in Simulator.

---

## Open Questions & Risks

### HIGH: iroh + iOS compatibility
`iroh 0.97` depends on `quinn` → `ring` (or `aws-lc-rs`) for crypto. Both `ring` and `aws-lc-rs` have C code that cross-compiles to iOS but may require:
- `IPHONEOS_DEPLOYMENT_TARGET` environment variable during build
- Correct `cargo` host/target configuration
- Potential issues with `ring`'s assembly on `x86_64-apple-ios` simulator

**Mitigation:** Test `cargo build --target aarch64-apple-ios-sim` early; if ring fails, pin to a known-good version or switch to `aws-lc-rs` feature.

### HIGH: mDNS entitlement on iOS
The `address-lookup-mdns` feature uses raw multicast sockets. Apple requires the
`com.apple.developer.networking.multicast` entitlement for this, and it must be explicitly
requested from Apple via their request form.

**Mitigation:** For initial port, disable mDNS (`iroh` without `address-lookup-mdns` feature)
and require users to use the full ticket URL (not just the compact ID). This is a functional
degradation but not a blocker.

### MEDIUM: Tokio on iOS
`tokio` with `rt-multi-thread` should work on iOS (uses pthreads). However, iOS may kill
background threads; the app should not expect long-lived background connections when backgrounded.

**Mitigation:** Handle app lifecycle (background/foreground) in SwiftUI and disconnect when backgrounding.

### MEDIUM: KMP Xcode integration complexity
The KMP `assembleSharedXCFramework` Gradle task requires AGP + KMP. The `shared` module must
not pull in Android-specific dependencies. Need to ensure `shared/build.gradle.kts` does NOT
apply `com.android.library` or use Android APIs in commonMain.

### LOW: Barcode format mapping
zxing on Android maps format → kind (0=barcode, 1=QR). AVFoundation uses different type strings.
The mapping must be correct to preserve compatibility with the receiver.

### LOW: JPEG quality / size
Android compresses to 1080px max, 85% JPEG. iOS `AVCapturePhotoOutput` produces higher-quality
images by default. Should apply equivalent downscaling.

---

## File Creation Summary

| File | Status |
|------|--------|
| `IOS_PORT_PLAN.md` | ✅ This file |
| `ios/rust/Cargo.toml` | To create |
| `ios/rust/src/lib.rs` | To create |
| `ios/rust/src/include/barcode_scanner.h` | To create |
| `build-ios.sh` | To create |
| `shared/build.gradle.kts` | To create |
| `shared/src/commonMain/kotlin/.../ShopInfo.kt` | To create |
| `shared/src/commonMain/kotlin/.../CardExtraction.kt` | To create |
| `shared/src/commonMain/kotlin/.../BarcodeState.kt` | To create |
| `android/settings.gradle.kts` (update) | To update |
| `android/app/build.gradle.kts` (update) | To update |
| `android/build.gradle.kts` (update) | To update |
| `ios/BarcodeScanner/ContentView.swift` | To create |
| `ios/BarcodeScanner/IrohBridge.swift` | To create |
| `ios/BarcodeScanner/BarcodeScannerView.swift` | To create |
| `ios/BarcodeScanner/BarcodeScanner-Bridging-Header.h` | To create |
| `ios/BarcodeScanner/Info.plist` | To create |
| `ios/BarcodeScanner/BarcodeScanner.entitlements` | To create |
| `ios/project.yml` | To create |
| `.github/workflows/build-ios.yml` | To create |

---

## Review Notes

### Migration paths for every Android-only API

| Android API | iOS Replacement | Notes |
|-------------|-----------------|-------|
| `zxing-embedded` ScanContract | `AVCaptureSession` + `AVMetadataOutput` | Different lifecycle, but same data |
| `AppCompatActivity` | `UIViewController` (via SwiftUI) | State machine moves to `@StateObject` |
| `lifecycleScope` | Swift `async/await` + `Task` | Equivalent concurrency model |
| `Dispatchers.IO` | `Task.detached(priority: .background)` | Same pattern |
| `ClipboardManager` | `UIPasteboard.general` | Direct equivalent |
| `Intent.ACTION_VIEW` | `UIApplication.open(_:)` | Direct equivalent |
| `BitmapFactory` + `Bitmap` | `UIImage` + `jpegData(compressionQuality:)` | Same capability |
| `LinearLayout` XML | SwiftUI `VStack`/`HStack` | Declarative, easier |
| `android.permission.CAMERA` | `NSCameraUsageDescription` in Info.plist | Permission requested at runtime |
| `android.permission.INTERNET` | No explicit permission needed on iOS | Automatic |

### Rust FFI portability
The JNI-specific code is fully isolated in `android/rust/src/lib.rs`. The `barcode-proto`
crate has zero JNI or Android dependencies — it's pure Rust with tokio and iroh. The new
`ios/rust/src/lib.rs` will share the same `SessionHandle` pattern and call the same
`barcode_proto::send_scan()` function. No JNI leaks into shared logic. ✅

### CI signing plan
The CI plan correctly separates the smoke test (simulator, no signing) from distribution
(device, requires secrets). Simulator builds use `CODE_SIGNING_ALLOWED=NO`. The secrets
table is complete. ✅

### Implementation order (no circular dependencies)
1. Rust C-FFI crate (depends only on barcode-proto — already exists)
2. `build-ios.sh` (depends on step 1)
3. KMP `shared` module (pure Kotlin, no platform deps)
4. Update Android Gradle to use `shared` (depends on step 3)
5. Swift iOS app (depends on steps 1 + 2 for XCFramework, step 3 for shared.xcframework)
6. Xcode project config (depends on step 5)
7. CI workflow (depends on all above)

No circular dependencies. ✅
