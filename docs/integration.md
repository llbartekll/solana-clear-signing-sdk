# Swift integration

Build the checkout using the [quickstart](../README.md#run-the-demo), then add
its root as a local Swift package and import `SolanaClearsign`. The examples
below assume your app already has instruction data and resource URLs.

## Inputs and providers

Construct `SolanaInstructionInput` from the program ID, raw instruction `Data`
and ordered `SolanaAccountMeta` values. Preserve account order, duplicates,
signer/writable flags and trailing metas. The SDK renders one instruction;
transaction parsing and deciding what to sign are the host's responsibility.

The high-level client accepts three providers:

| Provider | Supplies |
| --- | --- |
| `Srf39IdlSource` | IDL JSON and provenance for a program. `BundledSrf39IdlSource` is the file-backed implementation. |
| `SolanaAccountDataProvider` (optional) | Raw linked-account bytes and owner, for example from RPC or a snapshot. |
| `SolanaPresentationMetadataProvider` (optional) | `tokenMetadata(for:)` and `addressLabel(for:)`. `LocalTokenRegistry` is a bundled reference implementation. |

```swift
let client = SolanaClearSigningClient(
    idlSource: try BundledSrf39IdlSource(manifestURL: manifestURL),
    accountDataProvider: accountProvider,
    presentationProvider: try LocalTokenRegistry(url: registryURL)
)
let outcome = try await client.render(instruction: instruction)
```

A rendered result contains `canonical`, `idl`, `hints`, `presentation` and
`diagnostics`. Keep the original instruction available and handle errors per
instruction. `invalidate(programId:)` / `invalidateAll()` let the host discard
cached engines and rejected IDLs when its source changes.

For strict canonical output without the overlay:

```swift
let client = try SolanaClearSigningClient(idlJSON: idlJSON)
let display = try await client.display(
    instruction: instruction, accountProvider: accountProvider
)
```

The inline initializer parses the primary and additional programs eagerly.
This path is unpinned; an invalid base58/32-byte program ID throws `invalidInput`.
See [architecture](architecture.md#canonical-output-and-presentation) for the
separation between canonical output, hints and presentation.

## IDL manifest

`BundledSrf39IdlSource` reads a manifest whose `file` entries live alongside it.
This is the Subscriptions entry from the demo, with the digest of its current
bundled IDL:

```json
{
  "schemaVersion": 1,
  "sourceId": "bundle:srf39-manifest.json",
  "idls": [
    {
      "programId": "De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44",
      "file": "subscriptions-root.json",
      "sha256": "cc87288e11096693b615765856d4da686836d1f2e260fff3b0ba9409d55fa448",
      "version": "0.1.0"
    }
  ]
}
```

The digest covers the exact file bytes, including display metadata and SDK
extensions. Omitting it produces `idl_digest_unpinned`; the host must decide
whether to allow that. A program/digest match is not on-chain authority
verification. For updated demo IDLs, use the reproducible authoring process in
[conformance](../conformance/README.md#running-the-checks), which updates the
IDL and manifest together.

## Token metadata and local labels

Select the registry for the transaction's cluster. Entries use full mint or
account addresses as keys:

```json
{
  "schemaVersion": 1,
  "cluster": "devnet",
  "tokens": [
    {
      "mint": "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU",
      "symbol": "USDC",
      "name": "USD Coin",
      "decimals": 6,
      "tokenProgram": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
    }
  ],
  "labels": [
    {
      "address": "De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44",
      "label": "Subscriptions Program",
      "source": "app-curated"
    }
  ]
}
```

A custom presentation provider can use a cache or RPC-backed lookup instead.
Return `SolanaTokenMetadata(symbol:name:decimals:tokenProgram:)` for known
mints, `SolanaAddressLabel(label:source:)` for local names, or `nil` when unknown.
Metadata and aliases do not attest to a token issuer or address ownership.

Symbols must match `[A-Za-z0-9][A-Za-z0-9$._-]{0,15}`. Registry loading rejects
invalid symbols and duplicate entries. Annotations remain keyed by canonical
field index if the host reorders its preview. Respect `appliesToValue`: a raw
amount must not acquire a ticker that makes its base units look denominated.

## Token amount bindings

An instruction's display may bind a top-level `u64`/`i64` amount to a public-key
argument or named account. The amount must have an `amountNumberDisplayNode`.
The following is an instruction display fragment:

```json
{
  "kind": "instructionDisplayNode",
  "x-solana-clearsign": {
    "tokenAmounts": [
      {
        "amount": "expectedAmount",
        "mint": { "source": "argument", "name": "expectedMint" }
      }
    ]
  }
}
```

Use `"source": "account"` for a named instruction account. Invalid targets,
non-address arguments, duplicate bindings and unknown extension fields are
rejected at IDL load. This is an **SDK extension**, not a standard Codama field.

The existing `tokenMetadata(for:)` callback provides the decimals and symbol.
For example, raw `9990000` with six decimals produces `9.99`; the SDK uses
integer arithmetic. This path needs no mint-account fetch unless the strict
IDL separately requests one. The raw account provider serves that separate
IDL account-link path, returning bytes rather than pre-decoded decimals.

```swift
let amount = rendered.presentation.tokenAmount(for: fieldIndex)
let value = amount?.value ?? rendered.canonical.fields[fieldIndex].value
// amount.decimals == nil means raw; annotations provide the separate symbol.
```

For explicitly bound fields, `tokenAmounts` provides either a complete scaled
numeric value or the raw fallback. Callback decimals must agree with any
explicit IDL literal or resolved account scale. Existing token-program owner,
symbol and unit checks still apply. Missing metadata/decimals, rejected symbols
or conflicts keep the presented amount raw. Malformed linked account bytes
still fail the render.

Canonical fields, sentences and the required-account planner retain strict
reference behavior. The SDK never guesses amount/mint relationships from names
or account positions. `hints.amounts[].token` records the explicit relationship;
`amount_scale_unresolved` describes the presented amount for bound fields,
while `interpolated_intent_unavailable` still describes the canonical sentence.

## Results and fallback handling

| Situation | Result |
| --- | --- |
| No program IDL | `.unsupported(.idlNotFound)`; low-level display returns `nil`. |
| No matching instruction / undecodable instruction bytes | `.unsupported(.instructionNotRecognized)` / `.unsupported(.instructionDecodeFailed)`. |
| Bad IDL, wrong program or digest mismatch | Throws `idlRejected`; cached until invalidated. |
| IDL source failure | Throws `idlSourceFailed(detail:retryable:)`; not cached. |
| Missing linked account or unsatisfied injected scale | Rendered with the applicable raw fallback; dependent sentences may be suppressed. |
| Malformed linked account | Throws `accountDecode`. |
| Unknown token, rejected symbol or metadata conflict | Token annotation omitted; explicitly bound amount falls back to raw. |
| Out-of-range date or negative duration | Bare integer; `hints.times[i].formatted == false`. |

Use `SolanaDiagnostic.code`, not its free-text message, for application logic:

- `linked_account_unavailable`, `amount_scale_unresolved`;
- `interpolated_intent_unavailable`;
- `token_metadata_not_found`, `token_symbol_rejected`;
- `token_registry_decimals_mismatch`, `token_registry_owner_mismatch`;
- `unit_literal_conflicts_with_metadata`, `idl_digest_unpinned`.

`interpolated_intent_unavailable` is informational; the other codes above are
warnings. An unavailable sentence need not invalidate independently rendered
fields. Diagnostics stay available even when a preview can explain the value.

## Build and test notes

`Package.swift` references the ignored `target/ios/SolanaClearsignFFI.xcframework`
and `bindings/swift/generated/`. Run `scripts/build-xcframework.sh` before
resolving the package and after changing Rust or UniFFI interfaces. It builds
iOS device and both simulator architectures; the macOS slice matches the build
machine. Missing SDKs or Rust targets produce setup instructions before compilation.

For command-line demo tests, generate the project as in the
[quickstart](../README.md#run-the-demo), then select a device from
`xcrun simctl list devices available`:

```bash
xcodebuild -project ios-demo/ClearsignDemo.xcodeproj \
  -scheme ClearsignDemo \
  -destination 'platform=iOS Simulator,id=YOUR_SIMULATOR_UDID' \
  test CODE_SIGNING_ALLOWED=NO
```

Oracle setup uses npm to install pinned pnpm because the Corepack bundled with
CI's Node version has outdated signing keys for that release. If npm reports
an existing Corepack-managed `pnpm` binary, run `corepack disable pnpm` before
installing. Use a user-writable Node installation for global packages.
Dependencies need an initial download; subsequent fixture verification is
read-only and offline. Capture refresh commands are documented separately in
[conformance](../conformance/README.md#running-the-checks).
