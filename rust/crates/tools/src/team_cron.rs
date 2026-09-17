//! The team and cron tool domains: create/delete a team, create/delete/list
//! cron entries.
//!
//! Moved out of `lib.rs` as one slice (DEV-STRUCT-01). Bodies are verbatim.
//!
//! They share a module because the runtime crate groups them the same way
//! (`runtime::team_cron_registry`), and both are about scheduled or grouped
//! work rather than one-shot tool calls. The registries themselves are
//! process-global and live in [`crate::registries`].

use serde::Deserialize;
use serde_json::{json, Value};

use super::to_pretty_json;
// `run_team_create` also reads the *task* registry to assign the team to a task,
// which is the cross-domain read the shared core exists for.
use crate::registries::{global_cron_registry, global_task_registry, global_team_registry};
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn run_team_create(input: TeamCreateInput) -> Result<String, String> {
    let task_ids: Vec<String> = input
        .tasks
        .iter()
        .filter_map(|t| t.get("task_id").and_then(|v| v.as_str()).map(str::to_owned))
        .collect();
    let team = global_team_registry().create(&input.name, task_ids);
    // Register team assignment on each task
    for task_id in &team.task_ids {
        let _ = global_task_registry().assign_team(task_id, &team.team_id);
    }
    to_pretty_json(json!({
        "team_id": team.team_id,
        "name": team.name,
        "task_count": team.task_ids.len(),
        "task_ids": team.task_ids,
        "status": team.status,
        "created_at": team.created_at
    }))
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn run_team_delete(input: TeamDeleteInput) -> Result<String, String> {
    match global_team_registry().delete(&input.team_id) {
        Ok(team) => to_pretty_json(json!({
            "team_id": team.team_id,
            "name": team.name,
            "status": team.status,
            "message": "Team deleted"
        })),
        Err(e) => Err(e),
    }
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn run_cron_create(input: CronCreateInput) -> Result<String, String> {
    let entry =
        global_cron_registry().create(&input.schedule, &input.prompt, input.description.as_deref());
    to_pretty_json(json!({
        "cron_id": entry.cron_id,
        "schedule": entry.schedule,
        "prompt": entry.prompt,
        "description": entry.description,
        "enabled": entry.enabled,
        "created_at": entry.created_at
    }))
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn run_cron_delete(input: CronDeleteInput) -> Result<String, String> {
    match global_cron_registry().delete(&input.cron_id) {
        Ok(entry) => to_pretty_json(json!({
            "cron_id": entry.cron_id,
            "schedule": entry.schedule,
            "status": "deleted",
            "message": "Cron entry removed"
        })),
        Err(e) => Err(e),
    }
}

pub(crate) fn run_cron_list(_input: Value) -> Result<String, String> {
    let entries: Vec<_> = global_cron_registry()
        .list(false)
        .into_iter()
        .map(|e| {
            json!({
                "cron_id": e.cron_id,
                "schedule": e.schedule,
                "prompt": e.prompt,
                "description": e.description,
                "enabled": e.enabled,
                "run_count": e.run_count,
                "last_run_at": e.last_run_at,
                "created_at": e.created_at
            })
        })
        .collect();
    to_pretty_json(json!({
        "crons": entries,
        "count": entries.len()
    }))
}

#[derive(Debug, Deserialize)]
pub(crate) struct TeamCreateInput {
    name: String,
    tasks: Vec<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TeamDeleteInput {
    team_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CronCreateInput {
    schedule: String,
    prompt: String,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CronDeleteInput {
    cron_id: String,
}
