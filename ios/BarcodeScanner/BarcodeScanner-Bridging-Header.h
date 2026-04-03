// Bridging header: exposes the Rust C-FFI functions to Swift.
// The header lives in the compiled BarcodeScanner.xcframework, but we reference
// the source path here so Xcode can resolve it during development.
#include "../../ios/rust/src/include/barcode_scanner.h"
