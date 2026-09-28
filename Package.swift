// swift-tools-version:5.9
// Two-target package mirroring the reference lib: a binaryTarget XCFramework
// (built by scripts/build-xcframework.sh) + a thin source target with the
// generated bindings and the handwritten client. No business logic in Swift.
//
// `main` keeps `useLocalRustXCFramework = true` so local development resolves
// the XCFramework built into target/ios. The Swift release workflow flips it
// to `false` on the tagged release commit and fills in the published zip URL
// and checksum, so SwiftPM consumers of a tag pull the GitHub Release asset.
import PackageDescription

private let useLocalRustXCFramework = false

let package = Package(
    name: "SolanaClearsign",
    platforms: [.iOS(.v15), .macOS(.v13)],
    products: [
        .library(name: "SolanaClearsign", targets: ["SolanaClearsign"])
    ],
    targets: [
        useLocalRustXCFramework
            ? .binaryTarget(
                name: "SolanaClearsignFFI",
                path: "target/ios/SolanaClearsignFFI.xcframework"
            )
            : .binaryTarget(
                name: "SolanaClearsignFFI",
                url: "https://github.com/llbartekll/solana-clear-signing-sdk/releases/download/0.1.0/SolanaClearsignFFI.xcframework.zip",
                checksum: "d1f30b928e988aa7473f58bd385d0fc04b0dad96825407ae2d086ee884267402"
            ),
        .target(
            name: "SolanaClearsign",
            dependencies: ["SolanaClearsignFFI"],
            path: "bindings/swift",
            sources: [
                "generated/solana_clearsign.swift",
                "SolanaClearSigningClient.swift",
                "Srf39IdlSource.swift",
                "SolanaPresentation.swift",
                "SolanaRenderOutcome.swift",
            ]
        ),
        .testTarget(
            name: "SolanaClearsignTests",
            dependencies: ["SolanaClearsign"],
            path: "bindings/swift-tests"
        ),
    ]
)
