import Foundation
import SolanaClearsign

/// Friendlier forms of formatted SDK time hints. The demo retains the original
/// ISO 8601 / `HH:mm:ss` text in Technical details. Zero timestamps retain
/// their canonical value; this formatter never assigns a business meaning.
enum TimePresentation {
    static func text(for hint: SolanaTimeHint) -> String? {
        guard hint.formatted, let seconds = hint.seconds else { return nil }
        switch hint.display {
        case .duration: return duration(seconds: seconds)
        case .dateTime: return dateTime(seconds: seconds)
        }
    }

    /// The largest exact whole unit, without rounding a billing period.
    static func duration(seconds: Int64) -> String? {
        guard seconds > 0 else { return nil }
        let units: [(Int64, String)] = [(86_400, "day"), (3_600, "hour"), (60, "minute"), (1, "second")]
        for (size, name) in units where seconds % size == 0 {
            let count = seconds / size
            return "\(count) \(name)\(count == 1 ? "" : "s")"
        }
        return nil
    }

    /// The local calendar rendering of a non-zero timestamp.
    static func dateTime(seconds: Int64, timeZone: TimeZone = .autoupdatingCurrent) -> String? {
        guard seconds != 0 else { return nil }
        let formatter = DateFormatter()
        formatter.dateStyle = .medium
        formatter.timeStyle = .medium
        formatter.timeZone = timeZone
        return "\(formatter.string(from: Date(timeIntervalSince1970: TimeInterval(seconds)))) (local)"
    }
}
