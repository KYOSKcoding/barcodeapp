package com.example.barcodescanner

/**
 * JNI bridge to the Rust barcode-scanner library.
 *
 * All methods are blocking and must be called from a background thread.
 * The native library creates a tokio runtime internally.
 */
object IrohBridge {
    init {
        System.loadLibrary("barcode_scanner_android")
    }

    /**
     * Connects to a remote receiver using an EndpointTicket string.
     *
     * Returns an opaque session handle (non-zero on success, 0 on failure).
     */
    external fun connect(ticket: String): Long

    /**
     * Sends a scanned barcode/QR code to the connected receiver.
     *
     * [kind] is 0 for barcode, 1 for QR code.
     * [code] is the scanned string content.
     * [imageJpeg] is an optional JPEG image (can be empty).
     *
     * Returns true on success.
     */
    external fun sendScan(handle: Long, kind: Int, code: String, imageJpeg: ByteArray): Boolean

    /**
     * Returns true if the connection is still alive.
     */
    external fun isConnected(handle: Long): Boolean

    /**
     * Disconnects and frees the session handle.
     *
     * The handle must not be used after this call.
     */
    external fun disconnect(handle: Long)
}
