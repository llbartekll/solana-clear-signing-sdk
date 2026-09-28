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

private let useLocalRustXCFramework = true

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
                url: "https://github.com/llbartekll/solana-clear-signing-sdk/releases/download/0.0.0/SolanaClearsignFFI.xcframework.zip",
                checksum: "0000000000000000000000000000000000000000000000000000000000000000"
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
