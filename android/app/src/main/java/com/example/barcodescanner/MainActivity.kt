package com.example.barcodescanner

import android.graphics.Bitmap
import android.os.Bundle
import android.util.Log
import android.widget.Button
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.lifecycle.lifecycleScope
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanIntentResult
import com.journeyapps.barcodescanner.ScanOptions
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.ByteArrayOutputStream

private const val TAG = "BarcodeScanner"

class MainActivity : AppCompatActivity() {

    private enum class State {
        IDLE,
        SCANNING_TICKET,
        CONNECTING,
        READY,
        SCANNING_CODE,
        SCANNED,
        SENDING,
    }

    private var state = State.IDLE
    private var sessionHandle: Long = 0L
    private var lastScannedCode: String? = null
    private var lastScannedFormat: String? = null
    private var lastScannedImageJpeg: ByteArray? = null

    private lateinit var statusText: TextView
    private lateinit var codeText: TextView
    private lateinit var actionButton: Button
    private lateinit var disconnectButton: Button

    // Launcher for scanning the EndpointTicket QR code
    private val ticketScanLauncher = registerForActivityResult(ScanContract()) { result ->
        onTicketScanned(result)
    }

    // Launcher for scanning a barcode/QR code to send
    private val codeScanLauncher = registerForActivityResult(ScanContract()) { result ->
        onCodeScanned(result)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        statusText = findViewById(R.id.status_text)
        codeText = findViewById(R.id.code_text)
        actionButton = findViewById(R.id.action_button)
        disconnectButton = findViewById(R.id.disconnect_button)

        actionButton.setOnClickListener { onActionButtonClicked() }
        disconnectButton.setOnClickListener { onDisconnect() }

        updateUI()
    }

    override fun onDestroy() {
        super.onDestroy()
        if (sessionHandle != 0L) {
            val handle = sessionHandle
            sessionHandle = 0L
            lifecycleScope.launch(Dispatchers.IO) {
                IrohBridge.disconnect(handle)
            }
        }
    }

    private fun onActionButtonClicked() {
        when (state) {
            State.IDLE -> {
                state = State.SCANNING_TICKET
                updateUI()
                val options = ScanOptions().apply {
                    setDesiredBarcodeFormats(ScanOptions.QR_CODE)
                    setPrompt("Scan the connection ticket QR code")
                    setBeepEnabled(false)
                    setOrientationLocked(false)
                }
                ticketScanLauncher.launch(options)
            }
            State.READY -> {
                state = State.SCANNING_CODE
                updateUI()
                val options = ScanOptions().apply {
                    setDesiredBarcodeFormats(ScanOptions.ALL_CODE_TYPES)
                    setPrompt("Scan a barcode or QR code")
                    setBeepEnabled(true)
                    setOrientationLocked(false)
                    setBarcodeImageEnabled(true)
                }
                codeScanLauncher.launch(options)
            }
            State.SCANNED -> {
                sendLastScan()
            }
            else -> { /* ignore clicks in transitional states */ }
        }
    }

    private fun onTicketScanned(result: ScanIntentResult) {
        val ticket = result.contents
        if (ticket == null) {
            Log.w(TAG, "Ticket scan cancelled")
            state = State.IDLE
            updateUI()
            return
        }

        Log.i(TAG, "Ticket scanned, connecting...")
        state = State.CONNECTING
        updateUI()

        lifecycleScope.launch(Dispatchers.IO) {
            try {
                val handle = IrohBridge.connect(ticket)
                withContext(Dispatchers.Main) {
                    if (handle != 0L) {
                        sessionHandle = handle
                        state = State.READY
                        Log.i(TAG, "Connected successfully")
                    } else {
                        state = State.IDLE
                        Log.e(TAG, "Connection failed (handle=0)")
                    }
                    updateUI()
                }
            } catch (e: Exception) {
                Log.e(TAG, "Connection error", e)
                withContext(Dispatchers.Main) {
                    state = State.IDLE
                    statusText.text = "Connection failed: ${e.message}"
                    updateUI()
                }
            }
        }
    }

    private fun onCodeScanned(result: ScanIntentResult) {
        val code = result.contents
        val format = result.formatName

        if (code == null) {
            Log.w(TAG, "Code scan cancelled")
            state = State.READY
            updateUI()
            return
        }

        Log.i(TAG, "Scanned: [$format] $code")
        lastScannedCode = code
        lastScannedFormat = format

        // Extract barcode image from zxing result
        lastScannedImageJpeg = extractBarcodeImage(result)
        Log.i(TAG, "Image size: ${lastScannedImageJpeg?.size ?: 0} bytes")

        state = State.SCANNED
        updateUI()
    }

    /**
     * Extract the barcode image from the scan result as a low-res JPEG.
     * zxing-embedded stores the bitmap in the result intent when setBarcodeImageEnabled(true).
     */
    private fun extractBarcodeImage(result: ScanIntentResult): ByteArray? {
        try {
            val bitmap = result.barcodeImagePath?.let { path ->
                android.graphics.BitmapFactory.decodeFile(path)
            } ?: return null

            // Scale down to max 320px on the long side for low-res transmission
            val maxDim = 320
            val scale = maxDim.toFloat() / maxOf(bitmap.width, bitmap.height)
            val scaledBitmap = if (scale < 1.0f) {
                Bitmap.createScaledBitmap(
                    bitmap,
                    (bitmap.width * scale).toInt(),
                    (bitmap.height * scale).toInt(),
                    true
                )
            } else {
                bitmap
            }

            val stream = ByteArrayOutputStream()
            scaledBitmap.compress(Bitmap.CompressFormat.JPEG, 70, stream)
            val jpeg = stream.toByteArray()

            if (scaledBitmap !== bitmap) {
                scaledBitmap.recycle()
            }
            bitmap.recycle()

            return jpeg
        } catch (e: Exception) {
            Log.w(TAG, "Failed to extract barcode image", e)
            return null
        }
    }

    private fun sendLastScan() {
        val code = lastScannedCode ?: return
        val format = lastScannedFormat
        val handle = sessionHandle
        if (handle == 0L) return

        // Map zxing format name to our protocol kind
        val kind = if (format == "QR_CODE") 1 else 0
        val imageJpeg = lastScannedImageJpeg ?: ByteArray(0)

        state = State.SENDING
        updateUI()

        lifecycleScope.launch(Dispatchers.IO) {
            try {
                val success = IrohBridge.sendScan(handle, kind, code, imageJpeg)
                withContext(Dispatchers.Main) {
                    if (success) {
                        Log.i(TAG, "Scan sent successfully (image: ${imageJpeg.size} bytes)")
                        statusText.text = "Sent!"
                    } else {
                        Log.e(TAG, "Failed to send scan")
                        statusText.text = "Send failed"
                    }
                    state = State.READY
                    updateUI()
                }
            } catch (e: Exception) {
                Log.e(TAG, "Send error", e)
                withContext(Dispatchers.Main) {
                    statusText.text = "Send error: ${e.message}"
                    state = State.READY
                    updateUI()
                }
            }
        }
    }

    private fun onDisconnect() {
        val handle = sessionHandle
        if (handle != 0L) {
            sessionHandle = 0L
            lifecycleScope.launch(Dispatchers.IO) {
                IrohBridge.disconnect(handle)
            }
        }
        lastScannedCode = null
        lastScannedFormat = null
        lastScannedImageJpeg = null
        state = State.IDLE
        Log.i(TAG, "Disconnected")
        updateUI()
    }

    private fun updateUI() {
        // Show disconnect button when connected
        val connected = state in listOf(State.READY, State.SCANNING_CODE, State.SCANNED, State.SENDING)
        disconnectButton.visibility = if (connected) android.view.View.VISIBLE else android.view.View.GONE

        when (state) {
            State.IDLE -> {
                statusText.text = "Ready to connect"
                codeText.text = ""
                actionButton.text = "Start"
                actionButton.isEnabled = true
            }
            State.SCANNING_TICKET -> {
                statusText.text = "Scanning ticket..."
                actionButton.isEnabled = false
            }
            State.CONNECTING -> {
                statusText.text = "Connecting..."
                actionButton.isEnabled = false
            }
            State.READY -> {
                statusText.text = "Connected"
                codeText.text = lastScannedCode?.let { "Last: $it" } ?: ""
                actionButton.text = "Scan"
                actionButton.isEnabled = true
            }
            State.SCANNING_CODE -> {
                statusText.text = "Scanning code..."
                actionButton.isEnabled = false
            }
            State.SCANNED -> {
                statusText.text = "Scanned: ${lastScannedFormat ?: "unknown"}"
                codeText.text = lastScannedCode ?: ""
                actionButton.text = "Send"
                actionButton.isEnabled = true
            }
            State.SENDING -> {
                statusText.text = "Sending..."
                actionButton.isEnabled = false
            }
        }
    }
}
