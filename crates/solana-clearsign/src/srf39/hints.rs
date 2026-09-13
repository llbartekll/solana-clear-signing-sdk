//! Structural display hints derived from the IDL provide/inject graph.
//!
//! These types are an **SDK extension**, not part of sRFC 39 or of the Codama
//! reference output. They expose, in neutral terms, the wiring the strict
//! renderer already evaluates: which instruction account (if any) supplied the
//! scale of an amount, which fields were rendered where, and which linked
//! accounts were read. They never influence the canonical
//! [`InstructionDisplay`](super::InstructionDisplay) and are computed from the
//! same resolution pass, so building them triggers no additional account
//! fetches.
//!
//! The standard provide/inject hints avoid token vocabulary: an amount whose `decimals`
//! come from account `mint` is reported as "scale from account `mint`", and it
//! is the host's presentation layer that decides whether that address is a
//! known token. The optional `AmountHint.token` separately records the explicit
//! namespaced SDK binding; it also triggers no account fetching.

use serde::Serialize;

/// Everything the renderer learned about one instruction beyond the canonical
/// display. Field order is deterministic: arguments and accounts follow IDL
/// order, linked-account reads are sorted by address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstructionDisplayHints {
    /// The IDL instruction node name (camelCase, as written in the IDL).
    pub instruction_name: String,
    /// `true` when the IDL declares an `interpolatedIntent` template but the
    /// sentence was suppressed because a placeholder could not be resolved
    /// or referenced a degraded amount.
    pub interpolated_intent_suppressed: bool,
    /// One entry per top-level argument carrying an `amountNumberDisplayNode`,
    /// in IDL argument order.
    pub amounts: Vec<AmountHint>,
    /// One entry per top-level argument carrying a `dateTimeNumberDisplayNode`
    /// or `durationNumberDisplayNode`, in IDL argument order.
    pub times: Vec<TimeHint>,
    /// One entry per rendered public-key argument value, in IDL argument
    /// order; a public-key array contributes one entry per present element.
    pub public_key_arguments: Vec<PublicKeyArgumentHint>,
    /// Named instruction accounts in IDL order, followed by one entry per
    /// rendered remaining-account field.
    pub accounts: Vec<AccountHint>,
    /// Every linked account the renderer needed, sorted by address. `Missing`
    /// covers both "no provider supplied" and "provider returned nothing".
    pub linked_account_reads: Vec<LinkedAccountRead>,
}

/// How one amount argument was presented and where its inputs came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AmountHint {
    /// Index into `InstructionDisplay::fields`, or `None` when the argument
    /// was skipped from the fallback list.
    pub field_index: Option<usize>,
    /// The IDL argument name.
    pub argument: String,
    /// The struct field name when the amount is a member of a flattened
    /// struct argument; `None` for a top-level argument.
    pub member: Option<String>,
    /// The decoded integer as a decimal string, before any scaling.
    pub raw_value: String,
    /// `true` when the canonical value carries the ` (raw)` marker because the
    /// scale could not be resolved.
    pub degraded: bool,
    pub decimals: DecimalsSource,
    pub unit: UnitSource,
    /// Explicit SDK metadata binding from `display.x-solana-clearsign`.
    /// This never changes the strict renderer's scale or field value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<TokenHint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TokenHint {
    /// Mint selected by the IDL binding; absent when the referenced value is missing.
    pub mint: Option<String>,
}

/// How one date-time or duration argument was presented. The canonical text
/// is either the reference's ISO 8601 / `HH:mm:ss` form or, when the value is
/// outside what the reference can format, the bare integer without any
/// degradation marker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeHint {
    /// Index into `InstructionDisplay::fields`, or `None` when skipped.
    pub field_index: Option<usize>,
    /// The IDL argument name.
    pub argument: String,
    /// The struct field name inside a flattened struct argument, if any.
    pub member: Option<String>,
    /// The decoded integer as a decimal string.
    pub raw_value: String,
    pub display: TimeDisplay,
    /// The exact whole seconds (`raw_value / ticks_per_second`, rounded toward
    /// negative infinity), present only when the canonical layer formatted the
    /// value and the result fits an `i64`.
    pub seconds: Option<i64>,
    /// `true` when the canonical value is the formatted form rather than the
    /// bare integer.
    pub formatted: bool,
}

/// An argument (or array element) whose IDL type is `publicKeyTypeNode`. The
/// presentation layer may look the address up like it looks up account
/// fields; nothing about the argument's role is inferred from its name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicKeyArgumentHint {
    /// Index into `InstructionDisplay::fields`, or `None` when skipped.
    pub field_index: Option<usize>,
    /// The IDL argument name.
    pub argument: String,
    /// The struct field name inside a flattened struct argument, if any.
    pub member: Option<String>,
    /// The element index when the argument is an array of public keys.
    pub element: Option<usize>,
    /// The decoded base58 address.
    pub address: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TimeDisplay {
    /// Ticks since the Unix epoch.
    #[serde(rename_all = "camelCase")]
    DateTime { ticks_per_second: u32 },
    /// Elapsed ticks.
    #[serde(rename_all = "camelCase")]
    Duration { ticks_per_second: u32 },
}

/// Where the `decimals` input of an amount display resolves to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DecimalsSource {
    /// The display node declares no `decimals`; the renderer scales by zero.
    ImplicitZero,
    /// A literal `numberValueNode`. Values above 255 cannot scale and degrade.
    Literal { value: u64 },
    /// A field of a named instruction account, read through its `accountLink`.
    #[serde(rename_all = "camelCase")]
    AccountField {
        /// The instruction account name in the IDL.
        account: String,
        /// The base58 address bound to that account, when a meta was supplied.
        address: Option<String>,
        /// The linked `accountNode` name, or `None` when the instruction
        /// account carries no `accountLink` (the value can then never resolve).
        linked_account: Option<String>,
        /// The field path inside the decoded account data.
        path: String,
        /// The resolved scale exactly as the renderer used it
        /// (`u8::try_from(value).ok()`), or `None` when unresolved.
        resolved: Option<u8>,
    },
    /// The injection key has no provider and no fallback, forms a cycle, or
    /// resolves to a node the renderer cannot evaluate as a number.
    Unsatisfied,
}

/// Where the optional `unit` input of an amount display resolves to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UnitSource {
    /// The display node declares no `unit`.
    None,
    /// A literal `stringValueNode` authored in the IDL.
    Literal { value: String },
    /// A field of a named instruction account, read through its `accountLink`.
    #[serde(rename_all = "camelCase")]
    AccountField {
        account: String,
        address: Option<String>,
        linked_account: Option<String>,
        path: String,
        /// The resolved non-empty string, or `None` when unresolved.
        resolved: Option<String>,
    },
    /// The injection key has no provider and no fallback, forms a cycle, or
    /// resolves to a node the renderer cannot evaluate as a string.
    Unsatisfied,
}

/// One instruction account (named or remaining) and how it was rendered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountHint {
    /// Index into `InstructionDisplay::fields`, or `None` when skipped.
    pub field_index: Option<usize>,
    /// The IDL account name; for remaining accounts, the group's argument name.
    pub name: String,
    /// The bound base58 address, or `None` when no meta was supplied.
    pub address: Option<String>,
    /// The label as rendered in the fallback list (including `#n` suffixes).
    pub label: String,
    /// The linked `accountNode` name, if the account declares an `accountLink`.
    pub linked_account: Option<String>,
    /// `true` when the account's data was surfaced through provide/inject,
    /// i.e. the `whenInjected` rule would hide it.
    pub consumed: bool,
}

/// A linked account the renderer requested from the provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedAccountRead {
    pub address: String,
    pub status: LinkedAccountStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LinkedAccountStatus {
    /// The provider returned bytes; `owner` is transported, never validated.
    Fetched { owner: String, length: usize },
    /// No provider was supplied or the provider returned nothing.
    Missing,
}
