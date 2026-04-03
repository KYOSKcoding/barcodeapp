package com.example.barcodescanner

/**
 * Metadata for a supported gift-card shop.
 *
 * [digitCounts] is the set of digit counts (after stripping non-digits from the scanned code)
 * that map to this shop's card number format.
 */
data class ShopInfo(val name: String, val url: String, val digitCounts: List<Int>)

/**
 * Supported shops and the digit-count patterns that identify their gift cards.
 *
 * Kept in sync with receiver/src/main.rs and barcode-proto/src/lib.rs.
 */
val SHOPS: List<ShopInfo> = listOf(
    ShopInfo(
        name = "REWE",
        url = "https://kartenwelt.rewe.de/rewe-geschenkkarte.html",
        digitCounts = listOf(13, 39),
    ),
    ShopInfo(
        name = "DM",
        url = "https://www.dm.de/services/services-im-markt/geschenkkarten",
        digitCounts = listOf(24, 32),
    ),
    ShopInfo(
        name = "EDEKA",
        url = "https://evci.pin-host.com/evci/#/saldo",
        digitCounts = listOf(32),
    ),
    ShopInfo(
        name = "ALDI",
        url = "https://www.helaba.com/de/aldi/",
        digitCounts = listOf(20, 36, 38),
    ),
    ShopInfo(
        name = "LIDL",
        url = "https://www.lidl.de/c/lidl-geschenkkarten/s10007775",
        digitCounts = listOf(18, 20, 36, 38),
    ),
)
