import Foundation

enum AppConfig {
    static var alchemyAPIKey: String? {
        guard let raw = Bundle.main.object(forInfoDictionaryKey: "AlchemyAPIKey") as? String else {
            return nil
        }
        let key = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !key.isEmpty, !key.contains("$(") else { return nil }
        return key
    }
}
