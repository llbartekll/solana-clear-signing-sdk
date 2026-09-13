//! Golden tests for the structural hints.
//!
//! The hints are an SDK derivation, not reference output, so their goldens
//! live here (`testdata/hints/<fixture>.json`) rather than under
//! `conformance/`. Regenerate deliberately with `UPDATE_HINTS=1 cargo test`.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Mutex;

use crate::provider::AccountData;
use crate::provider::Srf39AccountProvider;
use crate::InstructionContext;

use super::conformance_tests::{fixture_provider, load_fixture, FixtureProvider, OwnedInstruction};
use super::hints::{DecimalsSource, LinkedAccountStatus, TimeDisplay, UnitSource};
use super::{DisplayMiss, DisplayResult, Srf39DisplayError, Srf39Engine};

const FIXTURES: &[&str] = &[
    "default-display",
    "transfer-checked",
    "spl-token-transfer-checked",
    "spl-token-instructions",
    "struct-argument",
    "inert-nodes",
    "defined-types",
    "signed-numbers",
    "date-time-display",
    "duration-display",
    "enum-display",
    "fixed-string",
    "fixed-array",
    "cross-program-link",
    "flatten-struct",
    "subscriptions",
];

/// Records every address requested from the wrapped fixture provider.
struct CountingProvider {
    inner: FixtureProvider,
    calls: Mutex<Vec<String>>,
}

impl Srf39AccountProvider for CountingProvider {
    fn resolve_account<'a>(
        &'a self,
        address: &'a str,
    ) -> Pin<Box<dyn Future<Output = Option<AccountData>> + Send + 'a>> {
        self.calls
            .lock()
            .expect("calls lock")
            .push(address.to_string());
        Srf39AccountProvider::resolve_account(&self.inner, address)
    }
}

#[test]
fn hints_match_goldens_for_every_fixture() {
    let update = std::env::var("UPDATE_HINTS").is_ok_and(|value| value == "1");
    for fixture_name in FIXTURES {
        let (root_json, fixture, _) = load_fixture(fixture_name);
        let engine = Srf39Engine::from_json(&root_json).expect("load fixture engine");
        let mut actual = serde_json::Map::new();
        for scenario in &fixture.scenarios {
            let owned = OwnedInstruction::from_scenario(&fixture, scenario);
            let metas = owned.metas();
            let instruction = InstructionContext {
                program_id: &owned.program_id,
                instruction_data: &owned.data,
                accounts: &metas,
                fee_payer: None,
            };
            let provider = CountingProvider {
                inner: fixture_provider(
                    scenario
                        .account_data
                        .as_ref()
                        .unwrap_or(&fixture.account_data),
                ),
                calls: Mutex::new(Vec::new()),
            };
            let source = scenario
                .fetch_accounts
                .then_some(&provider as &dyn Srf39AccountProvider);
            let result =
                pollster::block_on(engine.display_instruction_detailed(&instruction, source));
            let value = match (scenario.expected_error.as_deref(), result) {
                (None, Ok(DisplayResult::Rendered { hints, .. })) => {
                    // Building hints must not add provider traffic: with a
                    // provider, every needed address is requested exactly
                    // once; without one, nothing is requested at all.
                    let calls = provider.calls.lock().expect("calls lock").clone();
                    let mut reads: Vec<String> = hints
                        .linked_account_reads
                        .iter()
                        .map(|read| read.address.clone())
                        .collect();
                    reads.sort();
                    if scenario.fetch_accounts {
                        let mut sorted_calls = calls.clone();
                        sorted_calls.sort();
                        assert_eq!(
                            sorted_calls, reads,
                            "fixture {fixture_name}, scenario {}: provider calls vs reads",
                            scenario.name
                        );
                        assert_eq!(
                            calls.len(),
                            reads.len(),
                            "fixture {fixture_name}, scenario {}: each address fetched once",
                            scenario.name
                        );
                    } else {
                        assert!(
                            calls.is_empty(),
                            "fixture {fixture_name}, scenario {}: no provider, no calls",
                            scenario.name
                        );
                    }
                    serde_json::to_value(hints).expect("serialize hints")
                }
                (None, Ok(DisplayResult::Miss(_))) => serde_json::Value::Null,
                (Some("accountDecode"), Err(Srf39DisplayError::AccountDecode { .. })) => {
                    serde_json::json!({ "error": "accountDecode" })
                }
                (expected, result) => panic!(
                    "fixture {fixture_name}, scenario {}: expected {expected:?}, got {result:?}",
                    scenario.name
                ),
            };
            actual.insert(scenario.name.clone(), value);
        }
        let actual = serde_json::Value::Object(actual);
        let path = golden_path(fixture_name);
        if update {
            let mut rendered = serde_json::to_string_pretty(&actual).expect("render golden");
            rendered.push('\n');
            std::fs::write(&path, rendered).expect("write golden");
            continue;
        }
        let expected: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read golden {}: {error}", path.display())),
        )
        .expect("parse golden");
        assert_eq!(actual, expected, "hints golden for fixture {fixture_name}");
    }
}

#[test]
fn spl_token_hints_expose_the_scale_account_by_name_not_position() {
    let rendered = render_all("spl-token-instructions");

    let online = rendered.hints("transferChecked-online");
    assert_eq!(online.instruction_name, "transferChecked");
    assert!(!online.interpolated_intent_suppressed);
    assert_eq!(online.amounts.len(), 1);
    let amount = &online.amounts[0];
    assert_eq!(amount.argument, "amount");
    assert_eq!(amount.field_index, Some(0));
    assert_eq!(amount.raw_value, "1469");
    assert!(!amount.degraded);
    assert_eq!(
        amount.decimals,
        DecimalsSource::AccountField {
            account: "mint".to_string(),
            address: Some("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string()),
            linked_account: Some("mint".to_string()),
            path: "decimals".to_string(),
            resolved: Some(6),
        }
    );
    assert_eq!(amount.unit, UnitSource::None);
    let mint = online
        .accounts
        .iter()
        .find(|account| account.name == "mint")
        .expect("mint account hint");
    assert_eq!(mint.field_index, Some(1));
    assert_eq!(mint.linked_account.as_deref(), Some("mint"));
    assert!(mint.consumed);
    assert_eq!(online.linked_account_reads.len(), 1);
    assert_eq!(
        online.linked_account_reads[0].status,
        LinkedAccountStatus::Fetched {
            owner: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
            length: 82,
        }
    );

    let offline = rendered.hints("transferChecked-offline");
    assert!(offline.interpolated_intent_suppressed);
    assert!(offline.amounts[0].degraded);
    assert!(matches!(
        &offline.amounts[0].decimals,
        DecimalsSource::AccountField { resolved: None, .. }
    ));
    // Without a provider the account is still reported as needed-but-missing.
    assert_eq!(offline.linked_account_reads.len(), 1);
    assert_eq!(
        offline.linked_account_reads[0].status,
        LinkedAccountStatus::Missing
    );

    // mintToChecked declares `mint` at position 0: the hint follows the IDL
    // account name, never a fixed position.
    let mint_to = rendered.hints("mintToChecked-online");
    let DecimalsSource::AccountField {
        account, address, ..
    } = &mint_to.amounts[0].decimals
    else {
        panic!("mintToChecked scales from an account field");
    };
    assert_eq!(account, "mint");
    assert_eq!(mint_to.accounts[0].name, "mint");
    assert_eq!(address.as_deref(), mint_to.accounts[0].address.as_deref());

    for name in ["revoke-online", "closeAccount-online"] {
        let hints = rendered.hints(name);
        assert!(hints.amounts.is_empty(), "{name} has no amount");
        assert!(
            hints.linked_account_reads.is_empty(),
            "{name} reads nothing"
        );
    }
}

#[test]
fn real_transfer_checked_hints_cover_missing_multisig_trailing_and_owner_cases() {
    let rendered = render_all("spl-token-transfer-checked");

    let missing = rendered.hints("missingAccount");
    assert!(missing.amounts[0].degraded);
    assert_eq!(missing.linked_account_reads.len(), 1);
    assert_eq!(
        missing.linked_account_reads[0].status,
        LinkedAccountStatus::Missing
    );
    let mint = missing
        .accounts
        .iter()
        .find(|account| account.name == "mint")
        .expect("mint hint");
    assert_eq!(mint.field_index, Some(1));
    assert!(!mint.consumed);

    let multisig = rendered.hints("multisig");
    let remaining: Vec<_> = multisig
        .accounts
        .iter()
        .filter(|account| account.name == "multiSigners")
        .collect();
    assert_eq!(remaining.len(), 2);
    assert_eq!(remaining[0].label, "Multi Signers #1");
    assert_eq!(remaining[1].label, "Multi Signers #2");
    assert_eq!(remaining[0].field_index, Some(3));
    assert_eq!(remaining[1].field_index, Some(4));
    assert!(remaining
        .iter()
        .all(|account| account.linked_account.is_none()));

    let trailing = rendered.hints("trailingMintByte");
    assert!(matches!(
        trailing.linked_account_reads[0].status,
        LinkedAccountStatus::Fetched { length: 83, .. }
    ));
    assert!(matches!(
        &trailing.amounts[0].decimals,
        DecimalsSource::AccountField {
            resolved: Some(6),
            ..
        }
    ));

    let foreign_owner = rendered.hints("foreignOwner");
    assert_eq!(
        foreign_owner.linked_account_reads[0].status,
        LinkedAccountStatus::Fetched {
            owner: "11111111111111111111111111111111".to_string(),
            length: 82,
        }
    );
    assert!(matches!(
        &foreign_owner.amounts[0].decimals,
        DecimalsSource::AccountField {
            resolved: Some(6),
            ..
        }
    ));

    assert!(matches!(
        rendered.miss("foreignProgram"),
        DisplayMiss::ProgramNotInIdl
    ));
    assert!(matches!(
        rendered.miss("unknownDiscriminator"),
        DisplayMiss::InstructionNotIdentified
    ));
    assert!(matches!(
        rendered.miss("truncated"),
        DisplayMiss::InstructionDecodeFailed { .. }
    ));
}

#[test]
fn literal_unit_and_consumed_mint_are_reported() {
    let rendered = render_all("transfer-checked");
    let online = rendered.hints("online");
    assert_eq!(
        online.amounts[0].unit,
        UnitSource::Literal {
            value: "USDC".to_string()
        }
    );
    let mint = online
        .accounts
        .iter()
        .find(|account| account.name == "mint")
        .expect("mint hint");
    assert!(mint.consumed);
    assert_eq!(mint.field_index, None);

    let default = render_all("default-display");
    assert!(
        !default
            .hints("defaultDisplay")
            .interpolated_intent_suppressed
    );
}

#[test]
fn time_hints_report_seconds_only_when_formatted() {
    let durations = render_all("duration-display");
    let fortnight = durations.hints("fortnight");
    assert_eq!(fortnight.times.len(), 3);
    assert_eq!(fortnight.times[0].argument, "seconds");
    assert_eq!(
        fortnight.times[0].display,
        TimeDisplay::Duration {
            ticks_per_second: 1
        }
    );
    assert_eq!(fortnight.times[0].seconds, Some(1_209_600));
    assert!(fortnight.times[0].formatted);
    assert_eq!(fortnight.times[0].field_index, Some(0));
    assert_eq!(
        fortnight.times[1].display,
        TimeDisplay::Duration {
            ticks_per_second: 1000
        }
    );
    assert_eq!(fortnight.times[1].seconds, Some(1_209_600));

    let negative = durations.hints("negative");
    assert!(!negative.times[2].formatted);
    assert_eq!(negative.times[2].seconds, None);
    assert_eq!(negative.times[2].raw_value, "-1");

    let extremes = durations.hints("extremes");
    assert!(extremes.times[0].formatted, "u64::MAX still formats");
    assert_eq!(
        extremes.times[0].seconds, None,
        "seconds above i64::MAX are not exposed"
    );

    let dates = render_all("date-time-display");
    let over_max = dates.hints("overMax");
    assert!(!over_max.times[0].formatted);
    assert_eq!(over_max.times[0].seconds, None);
    let negative_second = dates.hints("negativeSecond");
    assert_eq!(
        negative_second.times[1].display,
        TimeDisplay::DateTime {
            ticks_per_second: 1000
        }
    );
    assert_eq!(
        negative_second.times[1].seconds,
        Some(-1),
        "floor division keeps sub-second negatives before the epoch"
    );
    assert!(dates.hints("epoch").amounts.is_empty());
}

#[test]
fn cross_program_links_resolve_like_local_ones() {
    const MINT: &str = "86xCnPeV69n6t3DnyGvkKobf9FdN2H9oiVDdaMpo2MMY";
    let rendered = render_all("cross-program-link");
    let online = rendered.hints("online");
    assert_eq!(
        online.linked_account_reads,
        vec![super::hints::LinkedAccountRead {
            address: MINT.to_string(),
            status: LinkedAccountStatus::Fetched {
                owner: "Vote111111111111111111111111111111111111111".to_string(),
                length: 9,
            },
        }]
    );
    assert!(matches!(
        &online.amounts[0].decimals,
        DecimalsSource::AccountField { linked_account: Some(link), resolved: Some(6), .. } if link == "mint"
    ));
    assert!(online.accounts[0].consumed);
    for scenario in ["danglingProgram", "danglingAccount", "localMissing"] {
        let hints = rendered.hints(scenario);
        assert!(
            hints.linked_account_reads.is_empty(),
            "{scenario}: nothing is fetched for a link that cannot decode"
        );
        assert!(hints.amounts[0].degraded, "{scenario}");
        assert!(!hints.accounts[0].consumed, "{scenario}");
    }

    let (root_json, fixture, _) = load_fixture("cross-program-link");
    let engine = Srf39Engine::from_json(&root_json).expect("engine");
    for scenario in &fixture.scenarios {
        if scenario.expected_error.is_some() || scenario.name == "truncated" {
            continue;
        }
        let owned = OwnedInstruction::from_scenario(&fixture, scenario);
        let metas = owned.metas();
        let instruction = InstructionContext {
            program_id: &owned.program_id,
            instruction_data: &owned.data,
            accounts: &metas,
            fee_payer: None,
        };
        let expected = if scenario.name == "addressedToAdditionalProgram" {
            Some(vec![])
        } else {
            // The reference planner lists the address whether or not the
            // link resolves; only the renderer skips what it cannot decode.
            Some(vec![MINT.to_string()])
        };
        assert_eq!(
            engine.required_accounts_for_display(&instruction),
            expected,
            "{}",
            scenario.name
        );
    }
}

#[test]
fn flattened_members_carry_their_field_names() {
    const MINT: &str = "86xCnPeV69n6t3DnyGvkKobf9FdN2H9oiVDdaMpo2MMY";
    let rendered = render_all("flatten-struct");
    let online = rendered.hints("online");
    assert_eq!(online.amounts.len(), 1);
    assert_eq!(online.amounts[0].argument, "args");
    assert_eq!(online.amounts[0].member.as_deref(), Some("price"));
    assert_eq!(online.amounts[0].field_index, Some(0));
    assert!(matches!(
        &online.amounts[0].decimals,
        DecimalsSource::AccountField {
            resolved: Some(6),
            ..
        }
    ));
    assert_eq!(online.times.len(), 1);
    assert_eq!(online.times[0].member.as_deref(), Some("expiresAt"));
    assert_eq!(online.times[0].field_index, Some(6));
    let keys: Vec<(Option<&str>, Option<usize>, Option<usize>)> = online
        .public_key_arguments
        .iter()
        .map(|hint| (hint.member.as_deref(), hint.element, hint.field_index))
        .collect();
    assert_eq!(
        keys,
        vec![
            (Some("owner"), None, Some(2)),
            (Some("keys"), Some(0), Some(3)),
            (Some("keys"), Some(1), Some(4)),
            (Some("mint"), None, None),
        ],
        "the `mint` field is hidden by `whenInjected` once the mint account is consumed"
    );
    assert_eq!(online.public_key_arguments[3].address, MINT);
    assert!(
        online.accounts[0].consumed,
        "flattened amounts consume through one level"
    );

    let offline = rendered.hints("offline");
    assert_eq!(offline.public_key_arguments[3].field_index, Some(6));
    assert!(offline.amounts[0].degraded);

    let none = rendered.hints("optionNone");
    assert!(
        none.amounts.is_empty(),
        "an absent struct renders as one `none` field"
    );
    assert!(
        none.accounts[0].consumed,
        "the static injection walk still consumes the mint"
    );

    let plain = rendered.hints("plainStruct");
    assert!(
        plain.amounts.is_empty(),
        "an amount inside an unflattened struct is not rendered"
    );
    assert!(!plain.accounts[0].consumed);
}

/// Approval instructions have no mint account from which the pinned reference
/// can read decimals. Their amounts stay raw even when account data is available.
#[test]
fn subscriptions_approvals_keep_unavailable_scales_raw() {
    let (root_json, fixture, expected) = load_fixture("subscriptions");
    let engine = Srf39Engine::from_json(&root_json).expect("engine");
    let rendered = render_all("subscriptions");
    let mut rendered_count = 0;
    for scenario in &fixture.scenarios {
        if expected[&scenario.name].is_null() || scenario.name.starts_with("transfer") {
            continue;
        }
        rendered_count += 1;
        let hints = rendered.hints(&scenario.name);
        assert!(
            hints.linked_account_reads.is_empty(),
            "{}: no linked account is read",
            scenario.name
        );
        let fields = &expected[&scenario.name]["fields"];
        for amount in &hints.amounts {
            assert_eq!(
                amount.decimals,
                DecimalsSource::Unsatisfied,
                "{}",
                scenario.name
            );
            assert!(amount.degraded, "{}", scenario.name);
            let index = amount.field_index.expect("amounts are rendered");
            assert!(
                fields[index]["value"]
                    .as_str()
                    .is_some_and(|value| value.ends_with(" (raw)")),
                "{}: {:?}",
                scenario.name,
                fields[index]
            );
        }
        let owned = OwnedInstruction::from_scenario(&fixture, scenario);
        let metas = owned.metas();
        let instruction = InstructionContext {
            program_id: &owned.program_id,
            instruction_data: &owned.data,
            accounts: &metas,
            fee_payer: None,
        };
        assert_eq!(
            engine.required_accounts_for_display(&instruction),
            Some(Vec::new()),
            "{}",
            scenario.name
        );
    }
    assert!(
        rendered_count >= 24,
        "authored scenarios plus captured devnet ones"
    );
    let recurring = rendered.hints("createRecurringDelegation-vendor-bytes");
    assert_eq!(recurring.amounts[0].argument, "amountPerPeriod");
    assert!(recurring.interpolated_intent_suppressed);
    assert_eq!(recurring.times.len(), 3);
    assert_eq!(recurring.times[0].seconds, Some(1_209_600));
    assert_eq!(recurring.times[1].seconds, Some(0));
    let subscribe = rendered.hints("subscribe-usdc");
    assert_eq!(subscribe.public_key_arguments.len(), 1);
    assert_eq!(subscribe.public_key_arguments[0].argument, "expectedMint");
    assert_eq!(subscribe.public_key_arguments[0].field_index, Some(1));
}

#[test]
fn subscriptions_transfers_request_only_the_instruction_mint() {
    let (root_json, fixture, _) = load_fixture("subscriptions");
    let engine = Srf39Engine::from_json(&root_json).expect("engine");
    let mut checked = 0;
    for scenario in &fixture.scenarios {
        if !scenario.name.starts_with("transfer") {
            continue;
        }
        let owned = OwnedInstruction::from_scenario(&fixture, scenario);
        let metas = owned.metas();
        let instruction = InstructionContext {
            program_id: &owned.program_id,
            instruction_data: &owned.data,
            accounts: &metas,
            fee_payer: None,
        };
        let expected = if scenario.name == "transferRecurring-missing-mint" {
            Vec::new()
        } else {
            vec!["4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU".to_string()]
        };
        assert_eq!(
            engine.required_accounts_for_display(&instruction),
            Some(expected),
            "{}",
            scenario.name
        );
        checked += 1;
    }
    assert!(checked > 0, "transfer scenarios must be exercised");
}

#[test]
fn required_accounts_match_the_reference_planner() {
    let (root_json, fixture, _) = load_fixture("spl-token-instructions");
    let engine = Srf39Engine::from_json(&root_json).expect("load fixture engine");
    let mut seen = BTreeMap::new();
    for scenario in &fixture.scenarios {
        let owned = OwnedInstruction::from_scenario(&fixture, scenario);
        let metas = owned.metas();
        let instruction = InstructionContext {
            program_id: &owned.program_id,
            instruction_data: &owned.data,
            accounts: &metas,
            fee_payer: None,
        };
        seen.insert(
            scenario.name.clone(),
            engine.required_accounts_for_display(&instruction),
        );
    }
    assert_eq!(
        seen["transferChecked-online"],
        Some(vec![
            "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string()
        ])
    );
    assert_eq!(
        seen["transferChecked-offline"],
        seen["transferChecked-online"]
    );
    assert_eq!(seen["revoke-online"], Some(Vec::new()));
    assert_eq!(seen["closeAccount-online"], Some(Vec::new()));

    let (root_json, fixture, _) = load_fixture("spl-token-transfer-checked");
    let engine = Srf39Engine::from_json(&root_json).expect("load fixture engine");
    let foreign = fixture
        .scenarios
        .iter()
        .find(|scenario| scenario.name == "foreignProgram")
        .expect("foreignProgram scenario");
    let owned = OwnedInstruction::from_scenario(&fixture, foreign);
    let metas = owned.metas();
    let instruction = InstructionContext {
        program_id: &owned.program_id,
        instruction_data: &owned.data,
        accounts: &metas,
        fee_payer: None,
    };
    assert_eq!(engine.required_accounts_for_display(&instruction), None);
}

struct RenderedFixture {
    results: BTreeMap<String, Result<DisplayResult, Srf39DisplayError>>,
}

impl RenderedFixture {
    fn hints(&self, scenario: &str) -> &super::hints::InstructionDisplayHints {
        match &self.results[scenario] {
            Ok(DisplayResult::Rendered { hints, .. }) => hints,
            other => panic!("scenario {scenario} did not render: {other:?}"),
        }
    }

    fn miss(&self, scenario: &str) -> &DisplayMiss {
        match &self.results[scenario] {
            Ok(DisplayResult::Miss(miss)) => miss,
            other => panic!("scenario {scenario} did not miss: {other:?}"),
        }
    }
}

fn render_all(fixture_name: &str) -> RenderedFixture {
    let (root_json, fixture, _) = load_fixture(fixture_name);
    let engine = Srf39Engine::from_json(&root_json).expect("load fixture engine");
    let mut results = BTreeMap::new();
    for scenario in &fixture.scenarios {
        let owned = OwnedInstruction::from_scenario(&fixture, scenario);
        let metas = owned.metas();
        let instruction = InstructionContext {
            program_id: &owned.program_id,
            instruction_data: &owned.data,
            accounts: &metas,
            fee_payer: None,
        };
        let provider = fixture_provider(
            scenario
                .account_data
                .as_ref()
                .unwrap_or(&fixture.account_data),
        );
        let source = scenario
            .fetch_accounts
            .then_some(&provider as &dyn Srf39AccountProvider);
        let result = pollster::block_on(engine.display_instruction_detailed(&instruction, source));
        results.insert(scenario.name.clone(), result);
    }
    RenderedFixture { results }
}

fn golden_path(fixture_name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/srf39/testdata/hints")
        .join(format!("{fixture_name}.json"))
}
