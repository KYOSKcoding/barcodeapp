package com.example.barcodescanner

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.net.Uri
import android.os.Bundle
import android.util.Log
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import androidx.lifecycle.lifecycleScope
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanIntentResult
import com.journeyapps.barcodescanner.ScanOptions
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.ByteArrayOutputStream

private const val TAG = "BarcodeScanner"

// ── Shop detection ────────────────────────────────────────────────────

private data class ShopInfo(val name: String, val url: String, val digitCounts: List<Int>)

private val SHOPS = listOf(
    ShopInfo("REWE",  "https://kartenwelt.rewe.de/rewe-geschenkkarte.html",          listOf(13, 39)),
    ShopInfo("DM",    "https://www.dm.de/services/services-im-markt/geschenkkarten", listOf(24, 32)),
    ShopInfo("EDEKA", "https://evci.pin-host.com/evci/#/saldo",                      listOf(32)),
    ShopInfo("ALDI",  "https://www.helaba.com/de/aldi/",                             listOf(20, 36, 38)),
    ShopInfo("LIDL",  "https://www.lidl.de/c/lidl-geschenkkarten/s10007775",         listOf(18, 20, 36, 38)),
)

private fun detectShops(code: String): List<ShopInfo> {
    val n = code.count { it.isDigit() }
    return SHOPS.filter { n in it.digitCounts }
}

/** Returns 1 number for most shops, 2 numbers for 32-digit (EDEKA/DM) cards. */
private fun extractCardNumbers(code: String): List<String> {
    val digits = code.filter { it.isDigit() }
    if (digits.isEmpty()) return emptyList()
    return when (digits.length) {
        39        -> listOf(digits.substring(0, 13))                              // REWE 39 → first 13
        38        -> listOf(digits.substring(18))                                 // ALDI/LIDL 38 → drop 18, keep 20
        36        -> listOf(digits.substring(18))                                 // ALDI/LIDL 36 → drop 18, keep 18
        32        -> listOf(digits.substring(11, 16), digits.substring(18))      // EDEKA/DM 32 → two numbers
        in 10..31 -> listOf(digits)                                              // DM 24, REWE 13, LIDL 18/20, ALDI 20
        else      -> emptyList()
    }
}

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
    private var isLocalMode = false
    private var sessionHandle: Long = 0L
    private var lastScannedCode: String? = null
    private var lastScannedFormat: String? = null
    private var lastScannedImageJpeg: ByteArray? = null
    private var lastTrimmedNumbers: List<String> = emptyList()
    private var lastDetectedShops: List<ShopInfo> = emptyList()
    private var lastRawDigits: String = ""
    private var sendJob: Job? = null
    private var countdownJob: Job? = null

    private lateinit var statusText: TextView
    private lateinit var rawCodeText: TextView
    private lateinit var codeText: TextView
    private lateinit var actionButton: Button
    private lateinit var scanPhoneButton: Button
    private lateinit var disconnectButton: Button
    private lateinit var copyButtonsContainer: LinearLayout
    private lateinit var shopLinksContainer: LinearLayout
    private lateinit var jumpToScanButton: Button
    private lateinit var disconnectActionButton: Button
    private lateinit var cancelSendButton: Button

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
        rawCodeText = findViewById(R.id.raw_code_text)
        codeText = findViewById(R.id.code_text)
        actionButton = findViewById(R.id.action_button)
        scanPhoneButton = findViewById(R.id.scan_phone_button)
        disconnectButton = findViewById(R.id.disconnect_button)
        copyButtonsContainer = findViewById(R.id.copy_buttons_container)
        shopLinksContainer = findViewById(R.id.shop_links_container)
        jumpToScanButton = findViewById(R.id.jump_to_scan_button)
        disconnectActionButton = findViewById(R.id.disconnect_action_button)
        cancelSendButton = findViewById(R.id.cancel_send_button)

        actionButton.setOnClickListener { onActionButtonClicked() }
        scanPhoneButton.setOnClickListener { onScanPhoneClicked() }
        disconnectButton.setOnClickListener { onBack() }
        disconnectActionButton.setOnClickListener { onDisconnect() }
        jumpToScanButton.setOnClickListener { onJumpToScan() }
        cancelSendButton.setOnClickListener { onBack() }

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

    private fun launchCodeScanner() {
        val options = ScanOptions().apply {
            setDesiredBarcodeFormats(ScanOptions.ALL_CODE_TYPES)
            setPrompt("Scan a barcode or QR code")
            setBeepEnabled(true)
            setOrientationLocked(false)
            setBarcodeImageEnabled(true)
        }
        codeScanLauncher.launch(options)
    }

    private fun onJumpToScan() {
        state = State.READY
        updateUI()
    }

    private fun onScanPhoneClicked() {
        isLocalMode = true
        state = State.SCANNING_CODE
        updateUI()
        launchCodeScanner()
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
                launchCodeScanner()
            }
            State.SCANNED -> {
                if (isLocalMode) {
                    state = State.SCANNING_CODE
                    updateUI()
                    launchCodeScanner()
                } else {
                    sendLastScan()
                }
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
            if (isLocalMode) {
                isLocalMode = false
                state = State.IDLE
            } else {
                state = State.READY
            }
            updateUI()
            return
        }

        Log.i(TAG, "Scanned: [$format] $code")
        lastScannedCode = code
        lastScannedFormat = format
        lastRawDigits = code.filter { it.isDigit() }
        lastTrimmedNumbers = extractCardNumbers(code)
        lastDetectedShops = detectShops(code)

        // Extract barcode image from zxing result
        lastScannedImageJpeg = extractBarcodeImage(result)
        Log.i(TAG, "Image size: ${lastScannedImageJpeg?.size ?: 0} bytes")

        state = State.SCANNED
        updateUI()
    }

    /**
     * Extract the barcode image from the scan result as a JPEG (max 1080px).
     * zxing-embedded stores the bitmap in the result intent when setBarcodeImageEnabled(true).
     */
    private fun extractBarcodeImage(result: ScanIntentResult): ByteArray? {
        try {
            val bitmap = result.barcodeImagePath?.let { path ->
                android.graphics.BitmapFactory.decodeFile(path)
            } ?: return null

            // Scale down to max 1080px on the long side
            val maxDim = 1080
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
            scaledBitmap.compress(Bitmap.CompressFormat.JPEG, 85, stream)
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

        val timeoutSecs = 15
        countdownJob = lifecycleScope.launch(Dispatchers.Main) {
            for (remaining in timeoutSecs downTo 1) {
                statusText.text = "Sending... $remaining"
                delay(1000)
            }
        }

        sendJob = lifecycleScope.launch(Dispatchers.IO) {
            try {
                val success = IrohBridge.sendScan(handle, kind, code, imageJpeg)
                withContext(Dispatchers.Main) {
                    countdownJob?.cancel()
                    countdownJob = null
                    if (state != State.SENDING) return@withContext
                    if (success) {
                        Log.i(TAG, "Scan sent successfully")
                        statusText.text = "Sent!"
                        state = State.READY
                    } else {
                        Log.e(TAG, "Failed to send scan (timeout or connection lost)")
                        statusText.text = "Send failed — reconnect required"
                        sessionHandle = 0L
                        state = State.IDLE
                    }
                    updateUI()
                }
            } catch (e: Exception) {
                Log.e(TAG, "Send error", e)
                withContext(Dispatchers.Main) {
                    countdownJob?.cancel()
                    countdownJob = null
                    if (state != State.SENDING) return@withContext
                    statusText.text = "Send error — reconnect required"
                    sessionHandle = 0L
                    state = State.IDLE
                    updateUI()
                }
            }
        }
    }

    private fun onBack() {
        countdownJob?.cancel(); countdownJob = null
        sendJob?.cancel()
        sendJob = null
        lastScannedCode = null
        lastScannedFormat = null
        lastScannedImageJpeg = null
        lastRawDigits = ""
        lastTrimmedNumbers = emptyList()
        lastDetectedShops = emptyList()
        isLocalMode = false
        state = State.IDLE
        Log.i(TAG, "Back to home (session kept)")
        updateUI()
    }

    private fun onDisconnect() {
        countdownJob?.cancel(); countdownJob = null
        sendJob?.cancel()
        sendJob = null
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
        lastRawDigits = ""
        lastTrimmedNumbers = emptyList()
        lastDetectedShops = emptyList()
        isLocalMode = false
        state = State.IDLE
        Log.i(TAG, "Disconnected")
        updateUI()
    }

    private fun copyNumber(text: String) {
        val cm = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        cm.setPrimaryClip(ClipData.newPlainText("card number", text))
        Toast.makeText(this, "Copied!", Toast.LENGTH_SHORT).show()
    }

    private fun updateCopyButtons() {
        copyButtonsContainer.removeAllViews()
        if (lastTrimmedNumbers.isEmpty()) {
            copyButtonsContainer.visibility = android.view.View.GONE
            return
        }
        copyButtonsContainer.visibility = android.view.View.VISIBLE
        copyButtonsContainer.orientation = android.widget.LinearLayout.VERTICAL

        if (lastRawDigits.length == 32 && lastTrimmedNumbers.size == 2) {
            // Full 32-digit number (DM uses this as card number)
            val fullNumText = android.widget.TextView(this).apply {
                text = lastRawDigits
                textSize = 14f
                setTextIsSelectable(true)
                setPadding(0, 0, 0, 4)
            }
            copyButtonsContainer.addView(fullNumText)

            val dmBtn = Button(this).apply {
                text = "Copy DM"
                setOnClickListener { copyNumber(lastRawDigits) }
            }
            copyButtonsContainer.addView(dmBtn)

            // Dashed divider
            val divider = android.widget.TextView(this).apply {
                text = "- - - - - - - - - - - - -"
                setTextColor(android.graphics.Color.GRAY)
                textSize = 12f
                setPadding(0, 8, 0, 8)
            }
            copyButtonsContainer.addView(divider)

            // Two EDEKA numbers on one line
            val edekaNumText = android.widget.TextView(this).apply {
                text = "${lastTrimmedNumbers[0]}  ${lastTrimmedNumbers[1]}"
                textSize = 14f
                setTextIsSelectable(true)
                setPadding(0, 0, 0, 4)
            }
            copyButtonsContainer.addView(edekaNumText)

            val edekaBtnRow = android.widget.LinearLayout(this).apply {
                orientation = android.widget.LinearLayout.HORIZONTAL
            }
            val edekaBtn1 = Button(this).apply {
                text = "Copy EDEKA 1"
                setOnClickListener { copyNumber(lastTrimmedNumbers[0]) }
            }
            val edekaBtn2 = Button(this).apply {
                text = "Copy EDEKA 2"
                layoutParams = android.widget.LinearLayout.LayoutParams(
                    android.widget.LinearLayout.LayoutParams.WRAP_CONTENT,
                    android.widget.LinearLayout.LayoutParams.WRAP_CONTENT
                ).also { it.marginStart = 8 }
                setOnClickListener { copyNumber(lastTrimmedNumbers[1]) }
            }
            edekaBtnRow.addView(edekaBtn1)
            edekaBtnRow.addView(edekaBtn2)
            copyButtonsContainer.addView(edekaBtnRow)
        } else {
            val multi = lastTrimmedNumbers.size > 1
            lastTrimmedNumbers.forEachIndexed { i, number ->
                val btn = Button(this).apply {
                    text = if (multi) "Copy ${i + 1}" else "Copy"
                    setOnClickListener { copyNumber(number) }
                }
                copyButtonsContainer.addView(btn)
            }
        }
    }

    private fun updateShopLinks() {
        shopLinksContainer.removeAllViews()
        if (lastDetectedShops.isEmpty()) {
            shopLinksContainer.visibility = android.view.View.GONE
            return
        }
        shopLinksContainer.visibility = android.view.View.VISIBLE
        for (shop in lastDetectedShops) {
            val btn = Button(this)
            btn.text = "🌐 ${shop.name}"
            btn.setOnClickListener {
                startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(shop.url)))
            }
            shopLinksContainer.addView(btn)
        }
    }

    private fun updateUI() {
        // "Scan on phone" button only visible on idle screen
        scanPhoneButton.visibility = if (state == State.IDLE) android.view.View.VISIBLE else android.view.View.GONE

        // "Jump to scanning" only visible on idle screen when already connected
        jumpToScanButton.visibility = if (state == State.IDLE && sessionHandle != 0L) android.view.View.VISIBLE else android.view.View.GONE

        // Cancel button only visible while sending
        cancelSendButton.visibility = if (state == State.SENDING) android.view.View.VISIBLE else android.view.View.GONE

        // Back button shown in READY and SCANNED (not SENDING — use Cancel instead)
        val showBack = state == State.READY || state == State.SCANNED
        disconnectButton.visibility = if (showBack) android.view.View.VISIBLE else android.view.View.GONE
        disconnectActionButton.visibility = if (showBack) android.view.View.VISIBLE else android.view.View.GONE

        // Raw code, copy buttons, and shop links only in SCANNED state
        rawCodeText.visibility = android.view.View.GONE
        copyButtonsContainer.visibility = android.view.View.GONE
        shopLinksContainer.visibility = android.view.View.GONE

        when (state) {
            State.IDLE -> {
                statusText.text = ""
                codeText.text = ""
                actionButton.text = "Connect to Receiver"
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
                codeText.text = ""
                actionButton.text = "Scan"
                actionButton.isEnabled = true
            }
            State.SCANNING_CODE -> {
                statusText.text = "Scanning..."
                actionButton.isEnabled = false
            }
            State.SCANNED -> {
                statusText.text = "Scanned: ${lastScannedFormat ?: "unknown"}"
                val raw = lastScannedCode ?: ""
                // Trimmed number(s) — for 32-digit show first (smaller) number only; otherwise trimmed
                val trimmedText = when {
                    lastRawDigits.length == 32 && lastTrimmedNumbers.isNotEmpty() -> lastTrimmedNumbers[0]
                    lastTrimmedNumbers.isNotEmpty() -> lastTrimmedNumbers.joinToString("\n")
                    else -> raw
                }
                codeText.text = trimmedText
                // Only show raw greyed code if trimming actually changed the value
                val noTrimming = lastTrimmedNumbers.size == 1 && lastTrimmedNumbers[0] == lastRawDigits
                if (noTrimming) {
                    rawCodeText.visibility = android.view.View.GONE
                } else {
                    rawCodeText.text = raw
                    rawCodeText.visibility = android.view.View.VISIBLE
                }
                // Dynamic copy buttons
                updateCopyButtons()
                // Shop links
                updateShopLinks()
                actionButton.text = if (isLocalMode) "Scan Again" else "Send"
                actionButton.isEnabled = true
            }
            State.SENDING -> {
                statusText.text = "Sending..."
                actionButton.isEnabled = false
            }
        }
    }
}
