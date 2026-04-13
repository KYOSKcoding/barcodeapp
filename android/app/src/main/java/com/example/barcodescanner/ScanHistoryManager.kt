package com.example.barcodescanner

import android.content.Context
import android.util.Log
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

private const val TAG = "ScanHistoryManager"
private const val INDEX_FILE = "index.json"
private const val SCANS_DIR = "scans"

object ScanHistoryManager {

    // ── In-memory checked state (not persisted) ───────────────────────
    // Keyed by scanned code string; value is the `hidden` flag from the receiver.
    // Populated by BackgroundSyncManager.  Cleared when the session ends.

    private val _checkedStates = MutableStateFlow<Map<String, Boolean>>(emptyMap())

    /** Emits the latest map of code → checked-on-receiver whenever it changes. */
    val checkedStatesFlow: StateFlow<Map<String, Boolean>> = _checkedStates.asStateFlow()

    /** Returns true if the receiver has marked this code as checked (hidden). */
    fun isCheckedOnReceiver(code: String): Boolean = _checkedStates.value[code] == true

    /** Called by BackgroundSyncManager after each successful sync poll. */
    fun updateCheckedStates(codeToChecked: Map<String, Boolean>) {
        _checkedStates.value = codeToChecked
    }

    /** Called by BackgroundSyncManager on disconnect to clear stale state. */
    fun clearCheckedStates() {
        _checkedStates.value = emptyMap()
    }

    private fun scansDir(context: Context): File {
        val dir = File(context.filesDir, SCANS_DIR)
        if (!dir.exists()) dir.mkdirs()
        return dir
    }

    private fun indexFile(context: Context) = File(scansDir(context), INDEX_FILE)

    fun addScan(context: Context, entry: ScanEntry, imageJpeg: ByteArray?) {
        // Write image file first
        if (imageJpeg != null && entry.imageFilename != null) {
            try {
                File(scansDir(context), entry.imageFilename).writeBytes(imageJpeg)
            } catch (e: Exception) {
                Log.w(TAG, "Failed to write image for ${entry.id}", e)
            }
        }

        // Append to index
        val index = loadIndex(context)
        index.put(entryToJson(entry))
        writeIndex(context, index)
    }

    fun updateStatus(context: Context, id: String, status: SendStatus) {
        val index = loadIndex(context)
        for (i in 0 until index.length()) {
            val obj = index.getJSONObject(i)
            if (obj.getString("id") == id) {
                obj.put("sendStatus", status.name)
                break
            }
        }
        writeIndex(context, index)
    }

    fun getAll(context: Context): List<ScanEntry> {
        val index = loadIndex(context)
        val entries = mutableListOf<ScanEntry>()
        for (i in 0 until index.length()) {
            try {
                entries.add(jsonToEntry(index.getJSONObject(i)))
            } catch (e: Exception) {
                Log.w(TAG, "Skipping malformed entry at index $i", e)
            }
        }
        // Newest first
        return entries.sortedByDescending { it.timestamp }
    }

    fun clearAll(context: Context) {
        val dir = scansDir(context)
        try {
            dir.listFiles()?.forEach { it.delete() }
        } catch (e: Exception) {
            Log.w(TAG, "clearAll: failed to delete files", e)
        }
        writeIndex(context, JSONArray())
    }

    fun deleteEntry(context: Context, id: String) {
        val dir = scansDir(context)
        val index = loadIndex(context)
        val newIndex = JSONArray()
        for (i in 0 until index.length()) {
            val obj = index.getJSONObject(i)
            if (obj.getString("id") == id) {
                val imgFile = obj.optString("imageFilename", "")
                if (imgFile.isNotEmpty()) {
                    File(dir, imgFile).delete()
                }
            } else {
                newIndex.put(obj)
            }
        }
        writeIndex(context, newIndex)
    }

    fun getImageFile(context: Context, imageFilename: String): File =
        File(scansDir(context), imageFilename)

    // ---- private helpers ----

    private fun loadIndex(context: Context): JSONArray {
        val file = indexFile(context)
        if (!file.exists()) return JSONArray()
        return try {
            JSONArray(file.readText())
        } catch (e: Exception) {
            Log.w(TAG, "Failed to parse index, starting fresh", e)
            JSONArray()
        }
    }

    private fun writeIndex(context: Context, index: JSONArray) {
        val file = indexFile(context)
        val tmp = File(file.parent, "${INDEX_FILE}.tmp")
        try {
            tmp.writeText(index.toString())
            tmp.renameTo(file)
        } catch (e: Exception) {
            Log.e(TAG, "Failed to write index", e)
            tmp.delete()
        }
    }

    private fun entryToJson(entry: ScanEntry): JSONObject {
        val trimmedArr = JSONArray()
        entry.trimmedNumbers.forEach { trimmedArr.put(it) }
        val shopsArr = JSONArray()
        entry.detectedShopNames.forEach { shopsArr.put(it) }
        return JSONObject().apply {
            put("id", entry.id)
            put("timestamp", entry.timestamp)
            put("code", entry.code)
            put("format", entry.format)
            put("rawDigits", entry.rawDigits)
            put("trimmedNumbers", trimmedArr)
            put("detectedShopNames", shopsArr)
            put("imageFilename", entry.imageFilename ?: "")
            put("sendStatus", entry.sendStatus.name)
        }
    }

    private fun jsonToEntry(obj: JSONObject): ScanEntry {
        val trimmedArr = obj.getJSONArray("trimmedNumbers")
        val trimmedNumbers = (0 until trimmedArr.length()).map { trimmedArr.getString(it) }
        val shopsArr = obj.getJSONArray("detectedShopNames")
        val shopNames = (0 until shopsArr.length()).map { shopsArr.getString(it) }
        val imgFilename = obj.optString("imageFilename", "").ifEmpty { null }
        val statusStr = obj.optString("sendStatus", "PENDING")
        val status = try { SendStatus.valueOf(statusStr) } catch (e: Exception) { SendStatus.PENDING }
        return ScanEntry(
            id = obj.getString("id"),
            timestamp = obj.getLong("timestamp"),
            code = obj.getString("code"),
            format = obj.getString("format"),
            rawDigits = obj.getString("rawDigits"),
            trimmedNumbers = trimmedNumbers,
            detectedShopNames = shopNames,
            imageFilename = imgFilename,
            sendStatus = status
        )
    }
}
