import Foundation

/// Swift wrapper around the Rust C-FFI barcode scanner library.
///
/// All blocking operations (connect, sendScan) are dispatched to a background
/// executor via `Task.detached` with `.background` priority so they don't block
/// the main actor.
///
/// The `handle` is an opaque pointer stored as `Int64` (same as `jlong` on Android).
/// It must be freed by calling `disconnect()`.
@MainActor
final class IrohBridge: ObservableObject {

    /// Non-zero when a live session exists.
    private var handle: Int64 = 0

    var isConnected: Bool {
        handle != 0 && barcode_scanner_is_connected(handle)
    }

    // MARK: – Connect

    /// Connects to the receiver at the given EndpointTicket string.
    ///
    /// Returns `true` on success.  Blocks the calling thread via the Rust
    /// Tokio runtime; **always call from a background context**.
    func connect(ticket: String) async -> Bool {
        let result: Int64 = await Task.detached(priority: .background) {
            ticket.withCString { ptr in
                barcode_scanner_connect(ptr)
            }
        }.value

        handle = result
        return result != 0
    }

    // MARK: – Send scan

    /// Sends a scanned code to the receiver.
    ///
    /// - Parameters:
    ///   - kind: 0 = barcode, 1 = QR code
    ///   - code: Raw scanned string
    ///   - imageJpeg: JPEG image data (may be empty)
    ///
    /// Returns `true` on success (scan ACKed within 15 seconds).
    func sendScan(kind: Int32, code: String, imageJpeg: Data) async -> Bool {
        let h = handle
        guard h != 0 else { return false }

        return await Task.detached(priority: .background) {
            code.withCString { codeCStr in
                if imageJpeg.isEmpty {
                    return barcode_scanner_send_scan(h, kind, codeCStr, nil, 0)
                } else {
                    return imageJpeg.withUnsafeBytes { rawBuf in
                        guard let basePtr = rawBuf.bindMemory(to: UInt8.self).baseAddress else {
                            return false
                        }
                        return barcode_scanner_send_scan(h, kind, codeCStr, basePtr, imageJpeg.count)
                    }
                }
            }
        }.value
    }

    // MARK: – Disconnect

    /// Closes the connection and frees the session handle.
    func disconnect() {
        let h = handle
        handle = 0
        if h != 0 {
            Task.detached(priority: .background) {
                barcode_scanner_disconnect(h)
            }
        }
    }
}
