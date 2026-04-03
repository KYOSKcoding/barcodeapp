import Foundation

/// Metadata for a supported gift-card shop.
struct ShopInfo {
    let name: String
    let url: URL
    let digitCounts: Set<Int>
}

/// Supported shops — mirrors the Kotlin shared module and receiver/src/main.rs.
let shops: [ShopInfo] = [
    ShopInfo(
        name: "REWE",
        url: URL(string: "https://kartenwelt.rewe.de/rewe-geschenkkarte.html")!,
        digitCounts: [13, 39]
    ),
    ShopInfo(
        name: "DM",
        url: URL(string: "https://www.dm.de/services/services-im-markt/geschenkkarten")!,
        digitCounts: [24, 32]
    ),
    ShopInfo(
        name: "EDEKA",
        url: URL(string: "https://evci.pin-host.com/evci/#/saldo")!,
        digitCounts: [32]
    ),
    ShopInfo(
        name: "ALDI",
        url: URL(string: "https://www.helaba.com/de/aldi/")!,
        digitCounts: [20, 36, 38]
    ),
    ShopInfo(
        name: "LIDL",
        url: URL(string: "https://www.lidl.de/c/lidl-geschenkkarten/s10007775")!,
        digitCounts: [18, 20, 36, 38]
    ),
]

/// Returns the shops whose digit-count patterns match the given code.
func detectShops(code: String) -> [ShopInfo] {
    let n = code.filter(\.isNumber).count
    return shops.filter { $0.digitCounts.contains(n) }
}

/// Extracts card number(s) from a raw scanned code.
///
/// Mirrors `barcode-proto/src/lib.rs::extract_card_number()` and the Kotlin shared module.
func extractCardNumbers(code: String) -> [String] {
    let digits = code.filter(\.isNumber)
    guard !digits.isEmpty else { return [] }

    switch digits.count {
    case 39:
        // REWE 39-digit barcode → first 13 digits
        return [String(digits.prefix(13))]
    case 38:
        // ALDI/LIDL 38 → drop first 18, keep last 20
        return [String(digits.dropFirst(18))]
    case 36:
        // ALDI/LIDL 36 → drop first 18, keep last 18
        return [String(digits.dropFirst(18))]
    case 32:
        // EDEKA/DM 32 → two separate numbers
        let idx11 = digits.index(digits.startIndex, offsetBy: 11)
        let idx16 = digits.index(digits.startIndex, offsetBy: 16)
        let idx18 = digits.index(digits.startIndex, offsetBy: 18)
        return [String(digits[idx11 ..< idx16]), String(digits[idx18...])]
    case 10 ... 31:
        // Covers REWE 13, DM 24, LIDL 18/20, ALDI 20, EDEKA 16 — keep as-is
        return [digits]
    default:
        return []
    }
}
