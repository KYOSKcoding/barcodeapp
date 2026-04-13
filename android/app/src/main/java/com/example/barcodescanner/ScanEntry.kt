package com.example.barcodescanner

enum class SendStatus { PENDING, SENT, FAILED, LOCAL }

data class ScanEntry(
    val id: String,
    val timestamp: Long,
    val code: String,
    val format: String,
    val rawDigits: String,
    val trimmedNumbers: List<String>,
    val detectedShopNames: List<String>,
    val imageFilename: String?,
    var sendStatus: SendStatus
)
