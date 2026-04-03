#pragma once
#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/**
 * Connects to a remote receiver using an EndpointTicket string.
 *
 * Returns an opaque session handle (non-zero on success, 0 on failure).
 * The returned handle must be freed by calling barcode_scanner_disconnect().
 *
 * @param ticket  Null-terminated EndpointTicket string (scanned from QR code).
 */
int64_t barcode_scanner_connect(const char *ticket);

/**
 * Sends a scanned barcode/QR code to the connected receiver.
 *
 * @param handle      Session handle returned by barcode_scanner_connect().
 * @param kind        0 = barcode, 1 = QR code.
 * @param code        Null-terminated scanned code string.
 * @param image_jpeg  Pointer to JPEG image bytes (may be NULL if no image).
 * @param image_len   Length of image_jpeg in bytes (0 if no image).
 *
 * Returns true on success (scan sent and ACKed within 15 seconds).
 */
bool barcode_scanner_send_scan(int64_t handle, int32_t kind, const char *code,
                               const uint8_t *image_jpeg, size_t image_len);

/**
 * Returns true if the connection is still alive.
 */
bool barcode_scanner_is_connected(int64_t handle);

/**
 * Disconnects and frees the session handle.
 * The handle must not be used after this call.
 */
void barcode_scanner_disconnect(int64_t handle);

#ifdef __cplusplus
}
#endif
