//! Scale resolution, sentence templates and remaining-account groups against
//! outputs recorded from the pinned reference (`testdata/reference`).

use serde::Deserialize;

use crate::{AccountData, AccountMeta, InstructionContext, Srf39AccountProvider};

use super::conformance_tests::FixtureProvider;
use super::{InstructionDisplay, Srf39Engine};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Corpus {
    mint: String,
    mint_decimals: u8,
    data_hex: String,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    root: serde_json::Value,
    accounts: Vec<(String, bool)>,
    expected: InstructionDisplay,
}

#[test]
fn resolution_matches_the_recorded_reference() {
    let corpus: Corpus =
        serde_json::from_str(include_str!("testdata/reference/resolution.json")).expect("corpus");
    let data: Vec<u8> = (0..corpus.data_hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&corpus.data_hex[at..at + 2], 16).expect("hex"))
        .collect();
    let mut provider = FixtureProvider::default();
    provider.accounts.insert(
        corpus.mint.clone(),
        AccountData {
            owner: corpus.mint.clone(),
            data: vec![corpus.mint_decimals],
        },
    );
    for case in corpus.cases {
        let engine = Srf39Engine::from_json(&case.root.to_string()).expect("engine");
        let metas: Vec<AccountMeta<'_>> = case
            .accounts
            .iter()
            .map(|(pubkey, is_signer)| AccountMeta {
                pubkey,
                is_signer: *is_signer,
                is_writable: false,
            })
            .collect();
        let instruction = InstructionContext {
            program_id: engine.program_id(),
            instruction_data: &data,
            accounts: &metas,
            fee_payer: None,
        };
        let actual = pollster::block_on(
            engine.display_instruction(&instruction, Some(&provider as &dyn Srf39AccountProvider)),
        )
        .expect("display");
        assert_eq!(actual, Some(case.expected), "{}", case.name);
    }
}
