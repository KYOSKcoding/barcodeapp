package com.example.barcodescanner

import android.app.AlertDialog
import android.content.Intent
import android.graphics.BitmapFactory
import android.graphics.Color
import android.os.Bundle
import android.view.View
import android.view.ViewGroup
import android.widget.ImageView
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.lifecycle.lifecycleScope
import androidx.recyclerview.widget.LinearLayoutManager
import androidx.recyclerview.widget.RecyclerView
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

class ScanHistoryActivity : AppCompatActivity() {

    private lateinit var recycler: RecyclerView
    private lateinit var emptyText: TextView
    private val entries = mutableListOf<ScanEntry>()
    private lateinit var adapter: ScanAdapter
    private var checkedStateJob: Job? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_scan_history)
        title = "Scan History"
        supportActionBar?.setDisplayHomeAsUpEnabled(true)

        recycler = findViewById(R.id.scans_recycler)
        emptyText = findViewById(R.id.empty_text)

        adapter = ScanAdapter(entries,
            onClick = { entry ->
                val intent = Intent(this, ScanDetailActivity::class.java)
                intent.putExtra(ScanDetailActivity.EXTRA_SCAN_ID, entry.id)
                startActivity(intent)
            },
            onLongClick = { entry ->
                AlertDialog.Builder(this)
                    .setTitle("Delete scan?")
                    .setMessage("Remove this scan from history?")
                    .setPositiveButton("Delete") { _, _ ->
                        lifecycleScope.launch(Dispatchers.IO) {
                            ScanHistoryManager.deleteEntry(this@ScanHistoryActivity, entry.id)
                            val fresh = ScanHistoryManager.getAll(this@ScanHistoryActivity)
                            withContext(Dispatchers.Main) {
                                entries.clear()
                                entries.addAll(fresh)
                                adapter.notifyDataSetChanged()
                                emptyText.visibility = if (entries.isEmpty()) View.VISIBLE else View.GONE
                            }
                        }
                    }
                    .setNegativeButton("Cancel", null)
                    .show()
            }
        )

        recycler.layoutManager = LinearLayoutManager(this)
        recycler.adapter = adapter
    }

    override fun onResume() {
        super.onResume()
        loadEntries()

        // Refresh list whenever the checked-state map changes (updated by BackgroundSyncManager).
        checkedStateJob = lifecycleScope.launch {
            ScanHistoryManager.checkedStatesFlow.collect {
                adapter.notifyDataSetChanged()
                updateSyncSubtitle()
            }
        }
    }

    override fun onPause() {
        super.onPause()
        checkedStateJob?.cancel()
        checkedStateJob = null
    }

    private fun loadEntries() {
        lifecycleScope.launch(Dispatchers.IO) {
            val fresh = ScanHistoryManager.getAll(this@ScanHistoryActivity)
            withContext(Dispatchers.Main) {
                entries.clear()
                entries.addAll(fresh)
                adapter.notifyDataSetChanged()
                emptyText.visibility = if (entries.isEmpty()) View.VISIBLE else View.GONE
                recycler.visibility = if (entries.isEmpty()) View.GONE else View.VISIBLE
                updateSyncSubtitle()
            }
        }
    }

    private fun updateSyncSubtitle() {
        val checkedCount = entries.count { ScanHistoryManager.isCheckedOnReceiver(it.code) }
        val pendingCount = entries.count {
            it.sendStatus == SendStatus.PENDING || it.sendStatus == SendStatus.FAILED
        }
        val isConnected = MainActivity.activeSessionHandle != 0L
        supportActionBar?.subtitle = when {
            !isConnected -> null
            pendingCount > 0 -> "$pendingCount pending"
            checkedCount > 0 -> "$checkedCount done on receiver"
            else -> "Synced"
        }
    }

    override fun onSupportNavigateUp(): Boolean {
        finish()
        return true
    }

    // ---- Adapter ----

    private inner class ScanAdapter(
        private val items: List<ScanEntry>,
        private val onClick: (ScanEntry) -> Unit,
        private val onLongClick: (ScanEntry) -> Unit
    ) : RecyclerView.Adapter<ScanAdapter.VH>() {

        inner class VH(val root: View) : RecyclerView.ViewHolder(root) {
            val thumbnail: ImageView = root.findViewById(R.id.scan_thumbnail)
            val codeText: TextView = root.findViewById(R.id.scan_code_text)
            val formatText: TextView = root.findViewById(R.id.scan_format_text)
            val timeText: TextView = root.findViewById(R.id.scan_time_text)
            val statusBadge: TextView = root.findViewById(R.id.scan_status_badge)
            val checkedIndicator: TextView = root.findViewById(R.id.scan_checked_indicator)
        }

        override fun onCreateViewHolder(parent: ViewGroup, viewType: Int): VH {
            val view = layoutInflater.inflate(R.layout.item_scan, parent, false)
            return VH(view)
        }

        override fun onBindViewHolder(holder: VH, position: Int) {
            val entry = items[position]

            // Thumbnail
            val imgFile = entry.imageFilename?.let { ScanHistoryManager.getImageFile(this@ScanHistoryActivity, it) }
            if (imgFile != null && imgFile.exists()) {
                lifecycleScope.launch(Dispatchers.IO) {
                    val bmp = BitmapFactory.decodeFile(imgFile.absolutePath)
                    withContext(Dispatchers.Main) {
                        holder.thumbnail.setImageBitmap(bmp)
                    }
                }
            } else {
                holder.thumbnail.setImageResource(android.R.drawable.ic_menu_camera)
            }

            // Code text: show trimmed if available
            val displayCode = when {
                entry.trimmedNumbers.isNotEmpty() -> entry.trimmedNumbers.joinToString(" / ")
                else -> entry.code
            }
            holder.codeText.text = displayCode
            holder.formatText.text = entry.format
            holder.timeText.text = formatTimestamp(entry.timestamp)

            // Status badge
            val (badgeText, badgeColor) = when (entry.sendStatus) {
                SendStatus.SENT -> "Sent" to Color.parseColor("#4CAF50")
                SendStatus.FAILED -> "Failed" to Color.parseColor("#F44336")
                SendStatus.PENDING -> "Pending" to Color.parseColor("#FF9800")
                SendStatus.LOCAL -> "Local" to Color.parseColor("#607D8B")
            }
            holder.statusBadge.text = badgeText
            holder.statusBadge.setBackgroundColor(badgeColor)

            // Checked-on-receiver indicator
            val isChecked = ScanHistoryManager.isCheckedOnReceiver(entry.code)
            holder.checkedIndicator.visibility = if (isChecked) View.VISIBLE else View.GONE
            holder.root.alpha = if (isChecked) 0.45f else 1.0f

            holder.root.setOnClickListener { onClick(entry) }
            holder.root.setOnLongClickListener { onLongClick(entry); true }
        }

        override fun getItemCount() = items.size
    }

    private fun formatTimestamp(ts: Long): String {
        val now = System.currentTimeMillis()
        val diff = now - ts
        return when {
            diff < 60_000 -> "Just now"
            diff < 3_600_000 -> "${diff / 60_000}m ago"
            diff < 86_400_000 -> "${diff / 3_600_000}h ago"
            else -> SimpleDateFormat("dd MMM HH:mm", Locale.getDefault()).format(Date(ts))
        }
    }
}
