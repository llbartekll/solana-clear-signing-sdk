# Architecture

The SDK renders one Solana instruction. It takes an enriched IDL, program
address, raw instruction bytes, ordered account metas and optional account
bytes. It returns canonical display fields plus SDK hints, presentation
metadata and diagnostics. Transaction parsing and signing policy belong to
the host.

For API examples see [integration](integration.md); for executable compatibility
and provenance see [conformance](../conformance/README.md).

## Runtime

```text
Host transaction parser
  → program id + instruction bytes + ordered account metas
  → Srf39Client: resolve IDL, parse, verify binding, cache
  → Srf39Engine: identify → decode → resolve account links / provide / inject
  → canonical InstructionDisplay + structural hints
  → presentation provider: token metadata and local labels
  → RenderOutcome + diagnostics
  → host preview and signing policy
```

The core contains no networking or program-specific decoder branches. Swift
exposes domain types and maps errors. The generated UniFFI types are compiled
into the same module and are therefore public symbols, but they are not a
supported API: use the handwritten types only.

The high-level client checks that program IDs are base58 32-byte keys. For an
IDL returned by a source, its primary program must match the requested program;
when a digest is supplied, SHA-256 must match the exact JSON bytes, and it is
checked before those bytes are parsed. Parsed engines, rejected IDLs and
"not found" answers (at most 256) are cached until invalidation, so a source
that learns a new program must be followed by an invalidation. Source failures
are not cached. An answer requested before an invalidation is used by the call
that requested it but is never cached. Inline IDLs are parsed eagerly, register
their primary and additional programs without a pinned digest, and are kept
when the cache is invalidated.

The low-level display API uses the same strict engine, without the presentation
overlay. A host can render instructions independently so an unsupported or
failed instruction does not hide the rest of a transaction.

## Canonical output and presentation

| Layer | Responsibility |
| --- | --- |
| Canonical display | Intents, interpolated sentences and ordered fields derived from the IDL, instruction and raw account data. Shared fixtures match the pinned oracle. |
| Structural hints | Field indexes, decoded addresses, exact time values, scale/unit sources and linked-account reads, derived from the same resolution pass. |
| Presentation overlay | Address labels, token annotations, numeric values for explicit amount/mint bindings, raw forms of amounts whose scale is contradicted and escaped forms of unsafe text, keyed by canonical field index. |
| Diagnostics | Machine-readable reasons for unavailable context, metadata conflicts and suppressed sentences. |

Hints never fetch extra accounts or infer meaning from field labels, program
names or a nearby mint. They are SDK output, not part of sRFC 39. Presentation
metadata does not modify canonical fields or sentences. Token metadata is
cached per mint for one render, shared by amount formatting and annotations.

`instruction.display.x-solana-clearsign.tokenAmounts` explicitly associates
an amount with a mint argument or account. It is a namespaced SDK extension;
the pinned Codama renderer ignores it. The SDK can scale the separately
presented amount using the application's token metadata callback. Without
such a binding, callback decimals only cross-check the strict scale.
[Integration](integration.md#token-amount-bindings) describes its schema and
fallback rules.

Amount formatting uses integer arithmetic. Date and duration formatting in
the canonical layer reproduces the pinned reference's numeric behavior,
including its double-precision conversion and ECMAScript date limits. Time
hints let the host present exact whole-unit durations where available.

## Supported model

The sRFC 39 proposal and the executable Codama schema use different node
names. This SDK consumes `standard: "codama"`, RootNode major version `1`,
with nodes such as `amountNumberDisplayNode` and `structFieldDisplayNode`.
Its compatibility target is the pinned implementation documented in
[conformance](../conformance/README.md), not every feature in the
[sRFC 39 discussion](https://github.com/solana-foundation/SRFCs/discussions/4).

Supported shapes include:

- Field and size discriminators; a single-candidate fallback when no
  discriminator is declared.
- `u8`, little-endian `u32`/`u64`/`i64`, public keys, `u8` booleans, fixed
  `u32`-prefixed options, structs, scalar `u8` enums, fixed-size UTF-8 strings
  and fixed-count arrays.
- Defined-type links, including additional programs; unresolved or recursive
  type links are rejected during loading.
- Positional accounts, remaining-account groups, local/cross-program account
  links, provided/injected values, literals and top-level account-field reads.
- Amount, date-time, duration and enum displays; `always`, `never` and
  `whenInjected` skipping; one-level struct flattening on top-level arguments.
- Flat `${data.name}` and `${accounts.name}` interpolation. Fallback fields
  appear as arguments, named accounts, then remaining accounts. Public-key
  arrays expand per element; other composite values use the reference's JSON
  representation, preserving declaration order and quoted `u64`/`i64` values.

PDA definitions, errors and account defaults are accepted only in positions
where the display runtime ignores them; those subtrees are opaque and may use
any node kind. Other unsupported node forms fail with a JSON path rather than
being silently reinterpreted; inside a defined type the path is that of the
definition. A document that does not have the expected JSON shape (for example,
a missing required key) fails with the deserializer's message, without a path.

Additional implementation constraints: `ticksPerSecond` must be a positive
integer when present; enum discriminators must equal variant indexes; strings
must have a fixed-size wrapper; fixed array counts must be positive. Account,
instruction-account, argument and struct-field names must be unique in their
scope. A field discriminator must be a number that can hold its default. Amount
decimals must fit `0..=255`: a larger literal is refused at load, and a larger
account or provided value renders raw. Boolean bytes and option tags other than 0 and 1
do not decode, since programs disagree on their meaning. Trailing
bytes are accepted where the reference accepts them; a size discriminator
is exact during identification.

The arguments of one instruction together, and each account type, may expand to
at most **65,536 decoded values**, including containers; struct field names
longer than 64 bytes count extra. Types may nest at most **32 levels**, counting
followed type links, and links may copy at most **16 MiB** of defined-type JSON
per document; both limits apply before anything is expanded. Validation counts nested arrays, structs and options,
including zero-byte elements, using checked arithmetic. The decoder checks
an array's full wire span before reserving storage and handles reservation
failure as a decode miss. These are SDK resource limits, not Codama rules.

## Subscriptions and current limitations

The demo bundles the derived Subscriptions IDL. Its additional Token Program
contains the full Mint account layout for linked reads, with no SPL Token
instructions. The derivation preserves wire layouts and adds reviewable display
metadata; [conformance](../conformance/README.md) records sources and regeneration.

| Case | Current behavior |
| --- | --- |
| `transferFixed`, `transferRecurring`, `transferSubscription` | Standard account links read `tokenMint.decimals`. Missing mint data leaves the amount raw. |
| `subscribe` | Canonical amount stays raw. The explicit `expectedAmount` → `expectedMint` binding enables callback scaling in the overlay. The captured example presents `10000000` as `10`, with a separate USDC annotation. |
| Fixed/recurring delegation approvals | Stay raw. Resolving the mint requires following `subscriptionAuthority.tokenMint` to another account, which the pinned display API cannot do. |
| Zero start/expiry timestamps | Retain the canonical epoch value. Sentinel meanings, where described, appear in the IDL-authored labels; Swift does not infer them. |
| Hour-based periods | Hours remain in the label; a fractional `ticksPerSecond` is outside the engine's subset. |

Other boundaries observed while decorating the program: nested account-field
paths and nested interpolation are unavailable; the derivation lifts argument
structs to make their fields addressable. Undeclared trailing accounts are
ignored unless a remaining-accounts group exists. Default field order cannot
express arbitrary preview ordering. These are limitations of the pinned model
or this implementation, not claims about all future Codama versions.

## Demo presentation

The parser combines static keys with loaded writable/readonly addresses,
derives signer/writable flags from the message header, and preserves positional
metas, duplicates and top-level instruction order. It ignores CPI instructions.
Each instruction has its own rendered, unsupported or failed result; the UI
does not label an entire transaction as clear-signed.

The preview preserves canonical field order and labels and displays the canonical
interpolated sentence when available. It uses SDK-presented amounts, local
address labels and canonical date/duration strings without interpreting
instruction names or mint addresses. Diagnostic messages and severity come
straight from Rust; Swift does not infer missing token identities or scales.
There are no program/digest-specific display rules in Swift. Hidden fields and
business descriptions belong to the IDL's display
metadata; unsupported descriptions are omitted rather than invented by the UI.
Technical details retain canonical values, raw bytes, account metas, diagnostics
and IDL provenance. Zero timestamps stay canonical, without inferred meanings
such as “no expiry”.

Local aliases retain the instruction role and expose the full address through
“Resolved in demo app from:”. They neither resolve account owners nor assert
real-world identities. Tests compare canonical output and hints with the
registry enabled and disabled.

RPC responses take precedence over bundled snapshots for known examples.
Account data is cached per inspection, including misses. Transaction and
account sources are reported independently: live, snapshot, mixed, missing,
or not required. Account snapshots are display context, not proof of state
at the historical transaction slot.

## Trust boundaries

The SDK verifies local program/digest binding, not on-chain IDL authority.
A digest proves which document was used, not whether its author is trustworthy.
The host must select IDLs, account sources and policies for unknown or
partially rendered instructions.

The strict engine decodes linked account bytes using the selected layout; it
does not enforce account-owner policy. Owners are passed through in hints and
used to cross-check token registry claims. Missing linked data can produce a
raw fallback; supplied malformed data produces an error. Like the reference,
the canonical layer ignores an account's size discriminators. When the length
contradicts one, the SDK reports `linked_account_layout_mismatch` and presents
the amount raw unless the host's token metadata confirms the scale.

Strings decoded from instruction or account bytes are attacker-controlled. The
canonical field keeps the reference's exact text (which drops NUL bytes), so it
may contain newlines, bidi controls or invisible characters. Such a field is
reported as `unsafe_display_text` with an escaped form in the overlay, and a
sentence that would include such text is withheld. Host-supplied token names,
labels and label sources with such characters, over 128 characters, or (labels)
reading as another address are dropped with a diagnostic.

Token symbols and names are host-curated metadata with no issuer attestation.
A readable render is not a transaction-validation or authorization verdict.
On-chain IDL discovery, Token-2022 metadata parsing, remote token lists,
transaction-level intent composition and signing controls are outside this MVP.

## Extending the engine

Add standard nodes with an oracle fixture, Rust conformance coverage and
meaningful decode/error cases. Preserve existing public semantics. Features
that the pinned model cannot represent should remain explicit presentation
extensions or be proposed upstream; do not silently reinterpret canonical
output.
