package com.example.barcodescanner

import android.content.Context
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

private const val TAG = "BackgroundSyncManager"

/**
 * Runs two background loops while a receiver session is active:
 *
 *  1. **Retry loop** — every 20 s, attempts to (re-)send any PENDING or FAILED
 *     scans (oldest first, up to 5 at a time).
 *
 *  2. **Sync-checked-state** — on connect and then every 30 s, sends a
 *     SYNC_POLL to the receiver and updates the in-memory checked-state map
 *     in ScanHistoryManager so the history screen can show the ✓ indicator.
 *
 * Call [start] right after connecting and [stop] on disconnect.
 */
class BackgroundSyncManager(
    private val context: Context,
    private val scope: CoroutineScope,
    private val getHandle: () -> Long,
) {
    private var job: Job? = null

    fun start() {
        job?.cancel()
        job = scope.launch(Dispatchers.IO) {
            // Pull all codes from the receiver and populate local history.
            syncAllFromReceiver()
            // Then sync checked states for whatever ended up in history.
            syncCheckedState()

            var ticksSinceSync = 0
            while (isActive) {
                delay(20_000L)
                retryPendingScans()
                ticksSinceSync++
                // Sync checked state every ~30 s (after every ~1.5 retry cycles).
                if (ticksSinceSync >= 2) {
                    syncCheckedState()
                    ticksSinceSync = 0
                }
            }
        }
    }

    fun stop() {
        job?.cancel()
        job = null
        ScanHistoryManager.clearCheckedStates()
    }

    // ── Sync all codes from receiver on connect ───────────────────────

    private suspend fun syncAllFromReceiver() {
        val handle = getHandle()
        if (handle == 0L) return

        val raw = try {
            IrohBridge.syncAll(handle)
        } catch (e: Exception) {
            Log.w(TAG, "syncAll JNI error", e)
            return
        }

        if (raw.isNullOrEmpty()) return

        val lines = raw.lines().filter { it.isNotEmpty() }
        if (lines.isEmpty()) return

        val now = System.currentTimeMillis()
        val total = lines.size
        val checkedMap = mutableMapOf<String, Boolean>()

        lines.forEachIndexed { idx, line ->
            val parts = line.split('\u001F')
            if (parts.size < 3) return@forEachIndexed

            val code = parts[0]
            val kindByte = parts[1].toIntOrNull() ?: 0
            val checked = parts[2] == "1"

            checkedMap[code] = checked

            val format = if (kindByte == 1) "QR_CODE" else "BARCODE"
            val rawDigits = code.filter { it.isDigit() }
            val trimmedNumbers = extractCardNumbers(code)
            val detectedShops = detectShops(code).map { it.name }
            // Assign timestamps so oldest entry (idx=0) is earliest; newest is closest to now.
            val timestamp = now - (total - 1 - idx) * 1_000L

            val entry = ScanEntry(
                id = "${now}_${idx}",
                timestamp = timestamp,
                code = code,
                format = format,
                rawDigits = rawDigits,
                trimmedNumbers = trimmedNumbers,
                detectedShopNames = detectedShops,
                imageFilename = null,
                sendStatus = SendStatus.SENT
            )
            ScanHistoryManager.addScan(context, entry, null)
        }

        Log.d(TAG, "Synced ${lines.size} entries from receiver")
        if (checkedMap.isNotEmpty()) {
            ScanHistoryManager.updateCheckedStates(checkedMap)
        }
    }

    // ── Retry pending/failed scans ────────────────────────────────────

    private suspend fun retryPendingScans() {
        val handle = getHandle()
        if (handle == 0L) return

        val toRetry = ScanHistoryManager.getAll(context)
            .filter { it.sendStatus == SendStatus.PENDING || it.sendStatus == SendStatus.FAILED }
            .sortedBy { it.timestamp }
            .take(5)

        if (toRetry.isEmpty()) return
        Log.d(TAG, "Retrying ${toRetry.size} pending/failed scan(s)")

        for (entry in toRetry) {
            val currentHandle = getHandle()
            if (currentHandle == 0L) break

            val imageFile = entry.imageFilename?.let {
                ScanHistoryManager.getImageFile(context, it)
            }
            val imageJpeg: ByteArray = if (imageFile != null && imageFile.exists()) {
                try { imageFile.readBytes() } catch (e: Exception) { ByteArray(0) }
            } else {
                ByteArray(0)
            }

            val kind = if (entry.format == "QR_CODE") 1 else 0
            val success = try {
                IrohBridge.sendScan(currentHandle, kind, entry.code, imageJpeg)
            } catch (e: Exception) {
                Log.w(TAG, "Retry send failed for ${entry.id}", e)
                false
            }

            if (success) {
                Log.i(TAG, "Background retry succeeded for ${entry.id}")
                ScanHistoryManager.updateStatus(context, entry.id, SendStatus.SENT)
            }
        }
    }

    // ── Sync checked state from receiver ─────────────────────────────

    private suspend fun syncCheckedState() {
        val handle = getHandle()
        if (handle == 0L) return

        val entries = ScanHistoryManager.getAll(context)
        if (entries.isEmpty()) return

        val codesNl = entries.joinToString("\n") { it.code }

        val raw = try {
            IrohBridge.syncCheckedState(handle, codesNl)
        } catch (e: Exception) {
            Log.w(TAG, "syncCheckedState JNI error", e)
            return
        }

        if (raw.isNullOrEmpty()) return

        val checkedMap: Map<String, Boolean> = raw.lines()
            .mapNotNull { line ->
                val idx = line.indexOf('\u001F')
                if (idx < 0) null
                else line.substring(0, idx) to (line.substring(idx + 1) == "1")
            }
            .toMap()

        Log.d(TAG, "Synced checked state: ${checkedMap.count { it.value }} checked")
        ScanHistoryManager.updateCheckedStates(checkedMap)
    }
}
