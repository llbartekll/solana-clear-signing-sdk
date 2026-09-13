//! Native implementation of the sRFC 39 per-instruction display layer.
//!
//! The node schema consumed here is the Codama v1 representation of sRFC 39
//! (`standard: "codama"`, spec major 1, display nodes as published from
//! `@codama/spec` 1.7.0 onwards). Its behavior is locked to the executable
//! fixtures under `conformance/` and can be expanded node-by-node.

mod binding;
mod client;
mod decode;
mod diagnostics;
mod display;
mod hints;
mod model;
mod presentation;
mod source;
mod token_amounts;

#[cfg(test)]
mod client_tests;
#[cfg(test)]
mod conformance_tests;
#[cfg(test)]
mod hints_tests;
#[cfg(test)]
mod presentation_tests;

use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

use crate::provider::Srf39AccountProvider;
use crate::InstructionContext;

use model::LoadedSrf39Idl;

pub use binding::{IdlBinding, IdlRejection};
pub use client::{
    RenderFailure, RenderOutcome, RenderedInstruction, Srf39Client, UnsupportedReason,
};
pub use diagnostics::DiagnosticSeverity;
pub use diagnostics::FormatDiagnostic;
pub use hints::{
    AccountHint, AmountHint, DecimalsSource, InstructionDisplayHints, LinkedAccountRead,
    LinkedAccountStatus, PublicKeyArgumentHint, TimeDisplay, TimeHint, TokenHint, UnitSource,
};
pub use presentation::{
    symbol_is_acceptable, AddressLabel, AnnotationKind, EmptyPresentationProvider, FieldAnnotation,
    PresentationMetadataProvider, PresentationOverlay, PresentedTokenAmount, TokenMetadata,
};
pub use source::{
    IdlOrigin, IdlProvenance, IdlResolution, IdlSourceError, ResolvedSrf39Idl, Srf39IdlSource,
    StaticIdlSource,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayField {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstructionDisplay {
    pub intent: String,
    pub interpolated_intent: Option<String>,
    pub fields: Vec<DisplayField>,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum Srf39IdlError {
    #[error("invalid sRFC 39 IDL JSON: {detail}")]
    InvalidJson { detail: String },

    #[error("invalid sRFC 39 IDL root: {detail}")]
    InvalidRoot { detail: String },

    #[error("unsupported sRFC 39 IDL node at {path}: {kind}")]
    UnsupportedIdlNode { path: String, kind: String },

    #[error("invalid sRFC 39 IDL schema: {detail}")]
    InvalidSchema { detail: String },
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum Srf39DisplayError {
    #[error("could not decode linked account '{account}': {detail}")]
    AccountDecode { account: String, detail: String },

    #[error("sRFC 39 display invariant failed: {detail}")]
    Internal { detail: String },
}

/// Why an instruction produced no display. All three are expected outcomes,
/// never errors: they are how the renderer says "not mine".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayMiss {
    /// The instruction's program id matches neither the primary program nor
    /// any additional program of the loaded IDL.
    ProgramNotInIdl,
    /// No instruction node's discriminators matched the instruction bytes.
    InstructionNotIdentified,
    /// An instruction node matched but its arguments could not be decoded
    /// from the supplied bytes.
    InstructionDecodeFailed { instruction: String },
}

/// The detailed result of rendering: either the canonical display together
/// with its structural hints, or the reason for a miss.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayResult {
    Rendered {
        display: InstructionDisplay,
        hints: InstructionDisplayHints,
    },
    Miss(DisplayMiss),
}

/// A validated, reusable sRFC 39 display engine.
#[derive(Debug, Clone)]
pub struct Srf39Engine {
    idl: LoadedSrf39Idl,
}

impl Srf39Engine {
    /// Parses and validates the supported sRFC 39 IDL subset once.
    pub fn from_json(json: &str) -> Result<Self, Srf39IdlError> {
        LoadedSrf39Idl::from_json(json).map(|idl| Self { idl })
    }

    /// Base58 public key of the IDL's primary program (`root.program.publicKey`).
    pub fn program_id(&self) -> &str {
        &self.idl.primary_program().public_key
    }

    /// Name of the IDL's primary program (`root.program.name`).
    pub fn program_name(&self) -> &str {
        &self.idl.primary_program().name
    }

    /// Public keys of `root.additionalPrograms`, in declaration order.
    pub fn additional_program_ids(&self) -> Vec<String> {
        self.idl
            .additional_programs()
            .iter()
            .map(|program| program.public_key.clone())
            .collect()
    }

    /// Renders one raw instruction. Identification and instruction-data decode
    /// misses return `Ok(None)`.
    pub async fn display_instruction(
        &self,
        instruction: &InstructionContext<'_>,
        provider: Option<&dyn Srf39AccountProvider>,
    ) -> Result<Option<InstructionDisplay>, Srf39DisplayError> {
        self.display_instruction_detailed(instruction, provider)
            .await
            .map(|result| match result {
                DisplayResult::Rendered { display, .. } => Some(display),
                DisplayResult::Miss(_) => None,
            })
    }

    /// Renders one raw instruction and reports the miss reason or the
    /// structural hints next to the canonical display. The display half is
    /// identical to [`Self::display_instruction`].
    pub async fn display_instruction_detailed(
        &self,
        instruction: &InstructionContext<'_>,
        provider: Option<&dyn Srf39AccountProvider>,
    ) -> Result<DisplayResult, Srf39DisplayError> {
        get_instruction_display(&self.idl, instruction, provider).await
    }

    /// The addresses whose account state the display would read, computed
    /// statically from the IDL and the instruction (parity with the reference
    /// `getRequiredAccountsForDisplay`). `None` when the instruction cannot
    /// be identified or decoded.
    pub fn required_accounts_for_display(
        &self,
        instruction: &InstructionContext<'_>,
    ) -> Option<Vec<String>> {
        let program = self.idl.program_by_address(instruction.program_id)?;
        let instruction_node = decode::identify_instruction(program, instruction.instruction_data)?;
        decode::decode_instruction(instruction_node, instruction.instruction_data)?;
        Some(display::required_accounts(instruction_node, instruction))
    }
}

pub(crate) async fn get_instruction_display(
    idl: &LoadedSrf39Idl,
    instruction: &InstructionContext<'_>,
    provider: Option<&dyn Srf39AccountProvider>,
) -> Result<DisplayResult, Srf39DisplayError> {
    let Some(program) = idl.program_by_address(instruction.program_id) else {
        return Ok(DisplayResult::Miss(DisplayMiss::ProgramNotInIdl));
    };
    let Some(instruction_node) =
        decode::identify_instruction(program, instruction.instruction_data)
    else {
        return Ok(DisplayResult::Miss(DisplayMiss::InstructionNotIdentified));
    };
    let Some(data) = decode::decode_instruction(instruction_node, instruction.instruction_data)
    else {
        return Ok(DisplayResult::Miss(DisplayMiss::InstructionDecodeFailed {
            instruction: instruction_node.name.clone(),
        }));
    };

    display::render(idl, program, instruction_node, &data, instruction, provider)
        .await
        .map(|output| DisplayResult::Rendered {
            display: output.display,
            hints: output.hints,
        })
}
