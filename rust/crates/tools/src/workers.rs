//! The worker tool domain: create, inspect, trust, prompt, restart and stop workers.
//!
//! Moved out of `lib.rs` as one domain (DEV-STRUCT-01). Bodies are verbatim.
//!
//! The worker registry is process-global and lives in [`crate::registries`].

use serde::Deserialize;

use runtime::worker_boot::WorkerReadySnapshot;

use super::{default_auto_recover_prompt_misdelivery, to_pretty_json};
use crate::registries::global_worker_registry;

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn run_worker_create(input: WorkerCreateInput) -> Result<String, String> {
    let worker = global_worker_registry().create(
        &input.cwd,
        &input.trusted_roots,
        input.auto_recover_prompt_misdelivery,
    );
    to_pretty_json(worker)
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn run_worker_get(input: WorkerIdInput) -> Result<String, String> {
    global_worker_registry()
        .get(&input.worker_id)
        .map_or_else(|| Err(format!("worker not found: {}", input.worker_id)), to_pretty_json)
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn run_worker_observe(input: WorkerObserveInput) -> Result<String, String> {
    let worker = global_worker_registry().observe(&input.worker_id, &input.screen_text)?;
    to_pretty_json(worker)
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn run_worker_resolve_trust(input: WorkerIdInput) -> Result<String, String> {
    let worker = global_worker_registry().resolve_trust(&input.worker_id)?;
    to_pretty_json(worker)
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn run_worker_await_ready(input: WorkerIdInput) -> Result<String, String> {
    let snapshot: WorkerReadySnapshot = global_worker_registry().await_ready(&input.worker_id)?;
    to_pretty_json(snapshot)
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn run_worker_send_prompt(input: WorkerSendPromptInput) -> Result<String, String> {
    let worker = global_worker_registry().send_prompt(&input.worker_id, input.prompt.as_deref())?;
    to_pretty_json(worker)
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn run_worker_restart(input: WorkerIdInput) -> Result<String, String> {
    let worker = global_worker_registry().restart(&input.worker_id)?;
    to_pretty_json(worker)
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn run_worker_terminate(input: WorkerIdInput) -> Result<String, String> {
    let worker = global_worker_registry().terminate(&input.worker_id)?;
    to_pretty_json(worker)
}

#[derive(Debug, Deserialize)]
pub(crate) struct WorkerCreateInput {
    cwd: String,
    #[serde(default)]
    trusted_roots: Vec<String>,
    #[serde(default = "default_auto_recover_prompt_misdelivery")]
    auto_recover_prompt_misdelivery: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WorkerIdInput {
    worker_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WorkerObserveInput {
    worker_id: String,
    screen_text: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WorkerSendPromptInput {
    worker_id: String,
    #[serde(default)]
    prompt: Option<String>,
}
