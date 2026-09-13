// swift-tools-version:5.9
// Two-target package mirroring the reference lib: a binaryTarget XCFramework
// (built by scripts/build-xcframework.sh) + a thin source target with the
// generated bindings and the handwritten client. No business logic in Swift.
import PackageDescription

let package = Package(
    name: "SolanaClearsign",
    platforms: [.iOS(.v15), .macOS(.v13)],
    products: [
        .library(name: "SolanaClearsign", targets: ["SolanaClearsign"])
    ],
    targets: [
        .binaryTarget(
            name: "SolanaClearsignFFI",
            path: "target/ios/SolanaClearsignFFI.xcframework"
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
