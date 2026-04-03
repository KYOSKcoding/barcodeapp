package com.example.barcodescanner

/**
 * Detects which shops a scanned code may belong to based on its digit count.
 *
 * Returns all shops whose [ShopInfo.digitCounts] include the number of digits in [code].
 */
fun detectShops(code: String): List<ShopInfo> {
    val n = code.count { it.isDigit() }
    return SHOPS.filter { n in it.digitCounts }
}

/**
 * Extracts card number(s) from a raw scanned code.
 *
 * The extraction rules mirror `barcode-proto/src/lib.rs::extract_card_number()`
 * and the reference implementation in `voucher-scanner.py`.
 *
 * Returns:
 * - An empty list if no digits are present.
 * - A single-element list for most formats.
 * - A two-element list for 32-digit EDEKA/DM cards (two separate numbers embedded).
 */
fun extractCardNumbers(code: String): List<String> {
    val digits = code.filter { it.isDigit() }
    if (digits.isEmpty()) return emptyList()
    return when (digits.length) {
        39        -> listOf(digits.substring(0, 13))               // REWE 39 → first 13
        38        -> listOf(digits.substring(18))                   // ALDI/LIDL 38 → drop 18, keep 20
        36        -> listOf(digits.substring(18))                   // ALDI/LIDL 36 → drop 18, keep 18
        32        -> listOf(digits.substring(11, 16), digits.substring(18)) // EDEKA/DM 32 → two numbers
        in 10..31 -> listOf(digits)                                // DM 24, REWE 13, LIDL 18/20, ALDI 20
        else      -> emptyList()
    }
}
