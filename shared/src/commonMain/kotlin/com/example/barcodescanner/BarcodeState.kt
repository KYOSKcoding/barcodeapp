package com.example.barcodescanner

/**
 * States of the scanner state machine, shared between Android and iOS.
 *
 * Transitions:
 *   IDLE → SCANNING_TICKET → CONNECTING → READY
 *   READY → SCANNING_CODE → SCANNED → SENDING → READY (on success)
 *                                             → IDLE    (on failure)
 */
enum class BarcodeState {
    /** Initial state; no connection established. */
    IDLE,

    /** Camera is open to scan the receiver's QR ticket. */
    SCANNING_TICKET,

    /** EndpointTicket parsed; connecting to receiver via iroh QUIC. */
    CONNECTING,

    /** Connected and ready to scan product barcodes/QR codes. */
    READY,

    /** Camera is open to scan a barcode or QR code. */
    SCANNING_CODE,

    /** Code scanned; showing extracted card number and shop links. */
    SCANNED,

    /** Sending scanned data to the receiver (15 s timeout). */
    SENDING,
}
