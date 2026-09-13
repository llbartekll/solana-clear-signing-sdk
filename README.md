# Solana Clear Signing SDK

Native Rust and Swift SDK that turns raw Solana instructions into readable
actions, amounts and addresses using enriched Codama IDLs and the
[sRFC 39 clear-signing model](https://github.com/solana-foundation/SRFCs/discussions/4).
It is intended for integration between a wallet's transaction parser and its
signing UI.

The runtime is Rust → UniFFI → Swift. JavaScript is used only by the test oracle.

## Subscriptions demo

The iOS app demonstrates **Subscriptions & Allowances on Devnet**:

- Enable Subscriptions, authorize recurring spending, and revoke a delegation.
- Additional examples for merchant subscriptions, collecting a recurring
  payment, fixed delegations, and revoking the program's token access.
- Readable previews with locally resolved address labels and token metadata;
  raw instructions, account metas, diagnostics and IDL provenance in details.

Examples are separate historical transactions, not one continuous subscription
lifecycle. The app tries public RPC first and falls back to committed snapshots
for known examples. No API key or funded wallet is needed. Manual signature
lookups require RPC access. The app does not create or sign transactions.

The same engine is also tested against SPL Token mainnet captures; those
examples are not bundled in the demo.

## Run the demo

You need **macOS**, full **Xcode** with the iOS SDK and a Simulator runtime,
**Git**, stable Rust installed through [rustup](https://rustup.rs/), and
[XcodeGen](https://github.com/yonaskolb/XcodeGen) (`brew install xcodegen`).
Launch Xcode once to complete setup and select it under **Settings → Locations
→ Command Line Tools**. Standalone Command Line Tools are insufficient.
The first build downloads dependencies.

```bash
git clone https://github.com/llbartekll/solana-clear-signing-sdk.git
cd solana-clear-signing-sdk
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios

./scripts/build-xcframework.sh
swift test
xcodegen generate --spec ios-demo/project.yml
open ios-demo/ClearsignDemo.xcodeproj
```

Select **ClearsignDemo**, choose an iOS Simulator and Run. Simulator builds
need no signing team. Start with **Enable Subscriptions** in the picker.

The SDK supports iOS 15+ and macOS 13+; the demo requires iOS 16+.
This is a source distribution: build the XCFramework and matching Swift
bindings before resolving the package. Integrate the checkout as a **local
Swift package**; adding the Git URL directly in SwiftPM is not supported yet.
See [integration](docs/integration.md) for setup details and callbacks.

## Use the SDK

Supply an IDL source, optional raw account-data provider, and optional
presentation metadata provider:

```swift
let client = SolanaClearSigningClient(
    idlSource: try BundledSrf39IdlSource(manifestURL: manifestURL),
    accountDataProvider: accountProvider,
    presentationProvider: try LocalTokenRegistry(url: registryURL)
)

switch try await client.render(instruction: instruction) {
case let .rendered(result):
    // Use result.canonical, result.presentation and result.diagnostics
    // to build the preview; retain the original instruction for inspection.
    print(result.canonical.intent)
case let .unsupported(reason):
    // Keep the raw instruction available to the user.
    print(reason)
}
```

Canonical fields come from the IDL and instruction/account data. Token metadata
and address labels are a separate presentation overlay. See
[integration](docs/integration.md) for amount formatting, manifest examples,
result semantics and fallback handling.

## Scope and trust

- Implements an explicit **subset of Codama v1**, tested against a pinned
  reference implementation; not a claim of full sRFC 39 compatibility.
- Verifies program binding and pinned IDL bytes. The host wallet decides
  whether the IDL, account sources and transaction are trustworthy.
- The `x-solana-clearsign` amount/mint binding is an **SDK extension**.
  `subscribe` can use callback decimals; delegation amounts currently stay raw.
- Local labels and token symbols are app-supplied metadata, not identity or
  issuer attestations. Missing scale information stays visible as raw units.
- Renders individual top-level instructions. It does not infer a transaction's
  overall intent, render CPI instructions, or authorize a signature.

[Architecture](docs/architecture.md) explains the supported model, trust
boundaries and current limitations.

## Tests

```bash
rustup component add rustfmt clippy
cargo fmt --all -- --check
cargo clippy --all-features --all-targets -- -D warnings
cargo test --all-features
```

The JavaScript oracle needs Node.js and npm (CI pins Node **20.18.0**):

```bash
npm install --global pnpm@10.15.1
pnpm --dir conformance/oracle install --frozen-lockfile
pnpm --dir conformance/oracle test
pnpm --dir conformance/oracle author:verify
pnpm --dir conformance/oracle devnet:verify
pnpm --dir conformance/oracle mainnet:verify
```

After generating the demo project, use **Product → Test** in Xcode on a
Simulator. The default suite includes the bundled captures. Live Devnet tests
are opt-in (`RUN_DEVNET_TESTS=1`); they submit transactions and need a funded
signer. The live lifecycle test covers initialization, delegation and
revocation, not payment collection.

See [conformance](conformance/README.md) for fixture coverage, provenance and
intentional regeneration commands. See [integration](docs/integration.md#build-and-test-notes)
for command-line iOS tests and setup troubleshooting.

## Documentation and license

- [Integration](docs/integration.md): Swift API, providers, IDL and registry setup.
- [Architecture](docs/architecture.md): runtime, supported schema and trust boundaries.
- [Conformance](conformance/README.md): oracle, captures and regression coverage.
- [MIT License](LICENSE). Third-party dependencies and upstream reference
  material retain their own licenses.
