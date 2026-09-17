//! The `LSP` tool domain: language-server queries for the workspace.
//!
//! Moved out of `lib.rs` (DEV-STRUCT-01). Body is verbatim; the registry is
//! process-global and lives in [`crate::registries`].

use serde::Deserialize;
use serde_json::json;

use super::to_pretty_json;
use crate::registries::global_lsp_registry;
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn run_lsp(input: LspInput) -> Result<String, String> {
    let registry = global_lsp_registry();
    let action = &input.action;
    let path = input.path.as_deref();
    let line = input.line;
    let character = input.character;
    let query = input.query.as_deref();

    match registry.dispatch(action, path, line, character, query) {
        Ok(result) => to_pretty_json(result),
        Err(e) => to_pretty_json(json!({
            "action": action,
            "error": e,
            "status": "error"
        })),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct LspInput {
    action: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    character: Option<u32>,
    #[serde(default)]
    query: Option<String>,
}
