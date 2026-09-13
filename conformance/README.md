# sRFC 39 conformance fixtures

This directory is the executable compatibility contract for the native
renderer. The JavaScript oracle is test-only and pins
`@codama/dynamic-instructions@0.4.1` with `codama@1.10.2`, corresponding to
release commit `34a299b3ee0a9621f9f22b9b1b3f4f2893ef6bfe`.

The engine grows [one standard node at a time](../docs/architecture.md#extending-the-engine).
Each node gets its own synthetic `testProgram` fixture whose `expected.json` is written by
the oracle *before* the Rust side is implemented; the real-program fixtures
then exercise the same nodes on captured bytes.

Synthetic fixture sets (one node or concern each):

- `default-display`: fallback naming and automatic argument field;
- `transfer-checked`: minimal linked-account and online/offline example;
- `struct-argument`: unflattened struct arguments as declaration-ordered JSON,
  `bigint` quoting, option shapes, and the reference `titleCase`;
- `inert-nodes`: PDA, error, and account-default nodes the display ignores;
- `defined-types`: `definedTypeLinkNode` inlining, including a link into an
  additional program's defined type;
- `signed-numbers`: `i64` arguments, scaling, and `i64` account fields;
- `date-time-display`, `duration-display`: `dateTimeNumberDisplayNode` and
  `durationNumberDisplayNode`, including ECMAScript date limits, expanded
  years, and the double-precision artefacts of the reference;
- `enum-display`: scalar enums by index (the pinned decoder's `getEnumCodec`
  shape), enum account fields, and option-wrapped displays;
- `fixed-string`: fixed-size UTF-8 strings (BOM, NUL, lossy decoding) and a
  string account field used as a unit;
- `fixed-array`: fixed-count arrays, public-key arrays expanded per element,
  arrays inside options and options inside arrays;
- `cross-program-link`: `accountLinkNode.program` into `additionalPrograms`,
  dangling links, and instructions addressed to an additional program;
- `flatten-struct`: `structFieldDisplayNode.flatten`/`flattenPrefix`,
  nested `skip`, and one-level injection collection.

Real-program fixture sets:

- `spl-token-transfer-checked`: real Token Program instruction and full Mint
  layout, including trailing bytes and multisig accounts;
- `spl-token-instructions`: the shared SPL Token IDL and cases used by the
  SDK and transaction-parser regression tests (hand-authored display metadata on the upstream wire layout,
  RootNode version `1.0.0`);
- `subscriptions`: the Subscriptions & Allowances program. `root.json` is
  **derived**, not hand-written: `oracle/author_subscriptions.mjs` applies
  two standard Codama visitors to the vendored program IDL and the display
  spec in `display.json`. It links the full Mint layout from the hash-pinned
  SPL Token IDL for transfer decimals. `provenance.json` records both source
  hashes, the visitors, and the resulting `rootSha256`. The visitors rebuild the root
  with the pinned toolchain's `CODAMA_VERSION`, so this RootNode says
  `1.8.0` while the pristine IDL says `1.0.0` (the engine compares the major
  only). Scenarios ending in `-devnet` are derived from the captured devnet
  transactions and are rewritten by `devnet:update`.

The Fixed Delegation demo uses a captured approval for a different mint than
Devnet USDC, recorded as `expectedMint` in the catalog and checked against the
authority snapshot. It does not receive a USDC label or inferred decimals. The
previous capture with System Program as spender remains in
`createFixedDelegation-system-spender-regression`.

Full Mainnet transaction captures and the test token registry live in
`captures/mainnet/`. They remain covered by `mainnet:verify` and the iOS parser
and rendering tests, but are not packaged in the Subscriptions demo.

The realistic fixture directories include provenance for the pinned upstream
IDL source and their frozen account data. The bundled RootNode hashes are also
verified by the offline snapshot checks.

Scenarios contain raw instruction bytes, ordered account metas, optional raw
account data, and expected display JSON. Field order is significant. An
`expectedError` is used only where linked-account decode must fail; normal
misses compare against `null`.

Only the canonical `InstructionDisplay` is covered by the oracle. The
structural hints and the presentation overlay produced by the high-level
client are SDK derivations: their goldens live in
`crates/solana-clearsign/src/srf39/testdata/hints/` and the Rust suite asserts
that the canonical half of a high-level render equals the strict renderer's
output for every scenario.

## Subscriptions regression coverage

These scenarios exercise the IDL engine used by the demo:

| Scenario | Coverage |
| --- | --- |
| Original recurring delegation capture | `subscriptions/createRecurringDelegation-vendor-bytes`; Rust also checks its program, instruction bytes, ordered addresses and authority data against `vendor/fixtures/recurring_delegation_devnet.json`. |
| Recurring/fixed amounts, offline data, period and expiry boundaries | `subscriptions` oracle cases and SDK hint goldens; `SubscriptionsFixtureTests` checks raw amounts, time hints and suppressed sentences through Swift. |
| Fixed, recurring and subscription payment amounts | Cross-program Mint links in all three `transfer*` oracle cases; offline/missing/corrupt mint, zero, maximum `u64`, changed decimals and owner mismatch exercise formatting and annotation boundaries in Rust. Swift checks canonical results and the decode error. |
| Captured recurring payment: 100000 base units → 0.1 USDC | `transferRecurring-devnet`, offline `devnet:verify`, Rust presentation and Swift SDK tests; iOS checks the capture with and without mint data and the raw-amount explanation. |
| Revoke without account lookups, with trailing accounts, with an address label | `revokeDelegation` and `revokeDelegation-with-receiver`; Swift checks offline rendering and labels kept separate from canonical addresses. |
| Unlimited allowance and revocation with unknown token metadata | Swift checks both authority actions with and without the token registry, including the unlimited-allowance wording. |
| Empty/truncated bytes, unknown program/discriminator, missing accounts | `subscriptions` oracle cases, Rust client outcome tests and Swift partial-display test. |
| Token lookup failure, malformed account data, metadata/owner disagreement | SPL Token online/offline/error oracle cases and Rust presentation tests; current async provider callbacks also run through Rust FFI and Swift tests. |
| Instruction layout agrees with the program IDL | `author:verify` reproduces the decorated IDL from the hash-pinned pristine source; the conformance tests decode captured instructions with it. |

Expectations follow the pinned Codama implementation. Missing account metas
can yield a partial display, while undecodable instruction bytes yield no
display; rendering is not transaction validation. Two-hop token lookups are
not part of this contract: delegation approvals and canonical subscribe amounts
remain raw. The SDK's [explicit token binding](../docs/integration.md#token-amount-bindings)
separately formats the presented subscribe amount; transfers read decimals from
their instruction mint account and stay raw when it is unavailable. Account-owner
policy remains a host responsibility; the presentation overlay suppresses token annotations
when fetched owners disagree with registry metadata.

## Running the checks

Install dependencies once (network access may be needed), then verify the
committed fixtures with offline, read-only checks:

```bash
pnpm --dir conformance/oracle install --frozen-lockfile
pnpm --dir conformance/oracle test
pnpm --dir conformance/oracle mainnet:verify
pnpm --dir conformance/oracle author:verify
pnpm --dir conformance/oracle devnet:verify
```

Regenerate oracle expectations only after an intentional compatibility change
(`update` is also a pnpm built-in, so call the script explicitly):

```bash
pnpm --dir conformance/oracle run update
```

Regenerate the derived Subscriptions RootNode after editing `display.json`:

```bash
pnpm --dir conformance/oracle author:subscriptions
pnpm --dir conformance/oracle run update
```

The authoring command updates the derived IDL, its provenance, the bundled
copy and the manifest digest together. If display labels change, also
regenerate the SDK's hint goldens with
`UPDATE_HINTS=1 cargo test hints_match_goldens_for_every_fixture`.

Refresh captures only when intended:

```bash
ALCHEMY_API_KEY=... pnpm --dir conformance/oracle mainnet:update
pnpm --dir conformance/oracle devnet:discover   # read-only scan, prints catalog entries
pnpm --dir conformance/oracle devnet:update     # public devnet RPC, no key
```

The updates validate expected signatures, slots, message versions, instruction
indexes, opcodes, account owners, lengths, and hashes before writing. CI uses
only the read-only commands.
