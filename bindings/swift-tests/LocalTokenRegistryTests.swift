import Foundation
import XCTest
@testable import SolanaClearsign

final class LocalTokenRegistryTests: XCTestCase {
    func testLoadsTokensAndLabels() async throws {
        let registry = try LocalTokenRegistry(data: Data(Self.registryJSON.utf8))

        XCTAssertEqual(registry.cluster, "mainnet-beta")
        XCTAssertEqual(registry.sourceId, "app-curated")
        let usdc = await registry.tokenMetadata(for: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")
        XCTAssertEqual(
            usdc,
            SolanaTokenMetadata(
                symbol: "USDC",
                name: "USD Coin",
                decimals: 6,
                tokenProgram: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
            )
        )
        let meme = await registry.tokenMetadata(for: "PAWSxhjTWVhgKmdoF1HbmSQjbPgu4uYCpZHYb4Nvttf")
        XCTAssertEqual(meme?.symbol, "PAWS")
        XCTAssertNil(meme?.decimals)
        let unknown = await registry.tokenMetadata(for: "11111111111111111111111111111111")
        XCTAssertNil(unknown)
        let label = await registry.addressLabel(for: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")
        XCTAssertEqual(label, SolanaAddressLabel(label: "SPL Token Program", source: "app-curated"))
    }

    func testRejectsDuplicateMint() {
        let json = Self.registryJSON.replacingOccurrences(
            of: "\"PAWSxhjTWVhgKmdoF1HbmSQjbPgu4uYCpZHYb4Nvttf\"",
            with: "\"EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v\""
        )
        XCTAssertThrowsError(try LocalTokenRegistry(data: Data(json.utf8))) { error in
            XCTAssertEqual(
                error as? LocalTokenRegistryError,
                .duplicateMint("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")
            )
        }
    }

    func testRejectsUnacceptableSymbol() {
        let spoof = Self.registryJSON.replacingOccurrences(of: "\"symbol\": \"USDC\"", with: "\"symbol\": \"USD\u{0421}\"")
        XCTAssertThrowsError(try LocalTokenRegistry(data: Data(spoof.utf8))) { error in
            guard case .unacceptableSymbol? = error as? LocalTokenRegistryError else {
                return XCTFail("Unexpected error: \(error)")
            }
        }
    }

    func testSymbolPolicyMatchesTheCore() {
        for ok in ["USDC", "wSOL", "BTC.b", "USD-1", "A", "x_y$", String(repeating: "A", count: 16)] {
            XCTAssertTrue(SolanaSymbolPolicy.isAcceptable(ok), ok)
        }
        for bad in ["", "USDC ", " USDC", "USD\u{0421}", "-x", ".x", "$x", String(repeating: "A", count: 17)] {
            XCTAssertFalse(SolanaSymbolPolicy.isAcceptable(bad), bad)
        }
    }

    private static let registryJSON = """
    {
      "schemaVersion": 1,
      "cluster": "mainnet-beta",
      "source": { "id": "app-curated", "retrievedAt": "2026-09-10" },
      "tokens": [
        {
          "mint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
          "symbol": "USDC",
          "name": "USD Coin",
          "decimals": 6,
          "tokenProgram": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
        },
        {
          "mint": "PAWSxhjTWVhgKmdoF1HbmSQjbPgu4uYCpZHYb4Nvttf",
          "symbol": "PAWS"
        }
      ],
      "labels": [
        { "address": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", "label": "SPL Token Program", "source": "app-curated" }
      ]
    }
    """
}
