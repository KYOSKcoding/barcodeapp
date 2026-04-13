package com.example.barcodescanner

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.BitmapFactory
import android.graphics.Color
import android.os.Bundle
import android.util.Log
import android.view.View
import android.widget.Button
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private const val TAG = "ScanDetailActivity"

class ScanDetailActivity : AppCompatActivity() {

    companion object {
        const val EXTRA_SCAN_ID = "scan_id"
    }

    private lateinit var detailImage: ImageView
    private lateinit var noImageText: TextView
    private lateinit var statusText: TextView
    private lateinit var formatText: TextView
    private lateinit var codeText: TextView
    private lateinit var rawCodeText: TextView
    private lateinit var copyButtonsContainer: LinearLayout
    private lateinit var sendButton: Button
    private lateinit var sendStatusText: TextView

    private var entry: ScanEntry? = null
    private var sendJob: Job? = null
    private var countdownJob: Job? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_scan_detail)
        title = "Scan Detail"
        supportActionBar?.setDisplayHomeAsUpEnabled(true)

        detailImage = findViewById(R.id.detail_image)
        noImageText = findViewById(R.id.detail_no_image_text)
        statusText = findViewById(R.id.detail_status_text)
        formatText = findViewById(R.id.detail_format_text)
        codeText = findViewById(R.id.detail_code_text)
        rawCodeText = findViewById(R.id.detail_raw_code_text)
        copyButtonsContainer = findViewById(R.id.detail_copy_buttons_container)
        sendButton = findViewById(R.id.detail_send_button)
        sendStatusText = findViewById(R.id.detail_send_status_text)

        val scanId = intent.getStringExtra(EXTRA_SCAN_ID) ?: run {
            finish()
            return
        }

        lifecycleScope.launch(Dispatchers.IO) {
            val loaded = ScanHistoryManager.getAll(this@ScanDetailActivity).find { it.id == scanId }
            withContext(Dispatchers.Main) {
                if (loaded == null) {
                    finish()
                    return@withContext
                }
                entry = loaded
                bindEntry(loaded)
            }
        }
    }

    override fun onSupportNavigateUp(): Boolean {
        finish()
        return true
    }

    override fun onDestroy() {
        super.onDestroy()
        countdownJob?.cancel()
        sendJob?.cancel()
    }

    private fun bindEntry(e: ScanEntry) {
        // Image
        val imgFile = e.imageFilename?.let { ScanHistoryManager.getImageFile(this, it) }
        if (imgFile != null && imgFile.exists()) {
            lifecycleScope.launch(Dispatchers.IO) {
                val bmp = BitmapFactory.decodeFile(imgFile.absolutePath)
                withContext(Dispatchers.Main) {
                    detailImage.setImageBitmap(bmp)
                    detailImage.visibility = View.VISIBLE
                    noImageText.visibility = View.GONE
                }
            }
        } else {
            detailImage.visibility = View.GONE
            noImageText.visibility = View.VISIBLE
        }

        // Status badge
        val (badgeText, badgeColor) = when (e.sendStatus) {
            SendStatus.SENT -> "Sent" to Color.parseColor("#4CAF50")
            SendStatus.FAILED -> "Failed" to Color.parseColor("#F44336")
            SendStatus.PENDING -> "Pending" to Color.parseColor("#FF9800")
            SendStatus.LOCAL -> "Local" to Color.parseColor("#607D8B")
        }
        statusText.text = badgeText
        statusText.setBackgroundColor(badgeColor)

        formatText.text = "${e.format} · ${e.timestamp.let { ts ->
            val diff = System.currentTimeMillis() - ts
            when {
                diff < 60_000 -> "Just now"
                diff < 3_600_000 -> "${diff / 60_000}m ago"
                diff < 86_400_000 -> "${diff / 3_600_000}h ago"
                else -> java.text.SimpleDateFormat("dd MMM HH:mm", java.util.Locale.getDefault()).format(java.util.Date(ts))
            }
        }}"

        // Code text: show trimmed or raw
        val trimmedText = when {
            e.rawDigits.length == 32 && e.trimmedNumbers.isNotEmpty() -> e.trimmedNumbers[0]
            e.trimmedNumbers.isNotEmpty() -> e.trimmedNumbers.joinToString("\n")
            else -> e.code
        }
        codeText.text = trimmedText

        val noTrimming = e.trimmedNumbers.size == 1 && e.trimmedNumbers[0] == e.rawDigits
        if (noTrimming || e.trimmedNumbers.isEmpty()) {
            rawCodeText.visibility = View.GONE
        } else {
            rawCodeText.text = e.code
            rawCodeText.visibility = View.VISIBLE
        }

        // Copy buttons
        buildCopyButtons(e)

        // Send button — show if not sent and a session is active
        val canSend = e.sendStatus != SendStatus.SENT &&
                e.sendStatus != SendStatus.LOCAL &&
                MainActivity.activeSessionHandle != 0L
        sendButton.visibility = if (canSend) View.VISIBLE else View.GONE
        sendButton.setOnClickListener { retrySend(e) }
    }

    private fun buildCopyButtons(e: ScanEntry) {
        copyButtonsContainer.removeAllViews()
        if (e.trimmedNumbers.isEmpty()) {
            copyButtonsContainer.visibility = View.GONE
            return
        }
        copyButtonsContainer.visibility = View.VISIBLE

        if (e.rawDigits.length == 32 && e.trimmedNumbers.size == 2) {
            val fullNumText = TextView(this).apply {
                text = e.rawDigits; textSize = 14f; setTextIsSelectable(true)
                setPadding(0, 0, 0, 4)
            }
            copyButtonsContainer.addView(fullNumText)
            val dmBtn = Button(this).apply {
                text = "Copy DM"
                setOnClickListener { copyNumber(e.rawDigits) }
            }
            copyButtonsContainer.addView(dmBtn)

            val divider = TextView(this).apply {
                text = "- - - - - - - - - - - -"
                setTextColor(Color.GRAY); textSize = 12f; setPadding(0, 8, 0, 8)
            }
            copyButtonsContainer.addView(divider)

            val edekaNumText = TextView(this).apply {
                text = "${e.trimmedNumbers[0]}  ${e.trimmedNumbers[1]}"
                textSize = 14f; setTextIsSelectable(true); setPadding(0, 0, 0, 4)
            }
            copyButtonsContainer.addView(edekaNumText)

            val edekaRow = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
            val edekaBtn1 = Button(this).apply {
                text = "Copy EDEKA 1"
                setOnClickListener { copyNumber(e.trimmedNumbers[0]) }
            }
            val edekaBtn2 = Button(this).apply {
                text = "Copy EDEKA 2"
                layoutParams = LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.WRAP_CONTENT,
                    LinearLayout.LayoutParams.WRAP_CONTENT
                ).also { it.marginStart = 8 }
                setOnClickListener { copyNumber(e.trimmedNumbers[1]) }
            }
            edekaRow.addView(edekaBtn1)
            edekaRow.addView(edekaBtn2)
            copyButtonsContainer.addView(edekaRow)
        } else {
            val multi = e.trimmedNumbers.size > 1
            e.trimmedNumbers.forEachIndexed { i, number ->
                val btn = Button(this).apply {
                    text = if (multi) "Copy ${i + 1}" else "Copy"
                    setOnClickListener { copyNumber(number) }
                }
                copyButtonsContainer.addView(btn)
            }
        }
    }

    private fun copyNumber(text: String) {
        val cm = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        cm.setPrimaryClip(ClipData.newPlainText("card number", text))
        Toast.makeText(this, "Copied!", Toast.LENGTH_SHORT).show()
    }

    private fun retrySend(e: ScanEntry) {
        val handle = MainActivity.activeSessionHandle
        if (handle == 0L) {
            Toast.makeText(this, "Not connected to receiver", Toast.LENGTH_SHORT).show()
            return
        }

        val imgFile = e.imageFilename?.let { ScanHistoryManager.getImageFile(this, it) }
        val imageJpeg: ByteArray = if (imgFile != null && imgFile.exists()) {
            try { imgFile.readBytes() } catch (ex: Exception) { ByteArray(0) }
        } else {
            ByteArray(0)
        }

        val kind = if (e.format == "QR_CODE") 1 else 0

        sendButton.isEnabled = false
        sendStatusText.visibility = View.VISIBLE
        sendStatusText.text = "Sending..."

        val timeoutSecs = 5
        countdownJob = lifecycleScope.launch(Dispatchers.Main) {
            for (remaining in timeoutSecs downTo 1) {
                sendStatusText.text = "Sending... $remaining"
                delay(1000)
            }
        }

        sendJob = lifecycleScope.launch(Dispatchers.IO) {
            try {
                val success = IrohBridge.sendScan(handle, kind, e.code, imageJpeg)
                withContext(Dispatchers.Main) {
                    countdownJob?.cancel(); countdownJob = null
                    if (success) {
                        Log.i(TAG, "Retry send succeeded for ${e.id}")
                        val updated = e.copy(sendStatus = SendStatus.SENT)
                        entry = updated
                        ScanHistoryManager.updateStatus(this@ScanDetailActivity, e.id, SendStatus.SENT)
                        statusText.text = "Sent"
                        statusText.setBackgroundColor(Color.parseColor("#4CAF50"))
                        sendButton.visibility = View.GONE
                        sendStatusText.text = "Sent successfully!"
                    } else {
                        Log.e(TAG, "Retry send failed for ${e.id}")
                        ScanHistoryManager.updateStatus(this@ScanDetailActivity, e.id, SendStatus.FAILED)
                        statusText.text = "Failed"
                        statusText.setBackgroundColor(Color.parseColor("#F44336"))
                        sendButton.isEnabled = true
                        sendStatusText.text = "Send failed — check connection"
                    }
                }
            } catch (ex: Exception) {
                Log.e(TAG, "Retry send error for ${e.id}", ex)
                withContext(Dispatchers.Main) {
                    countdownJob?.cancel(); countdownJob = null
                    ScanHistoryManager.updateStatus(this@ScanDetailActivity, e.id, SendStatus.FAILED)
                    statusText.text = "Failed"
                    statusText.setBackgroundColor(Color.parseColor("#F44336"))
                    sendButton.isEnabled = true
                    sendStatusText.text = "Send error: ${ex.message}"
                }
            }
        }
    }
}
