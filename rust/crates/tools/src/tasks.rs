//! The task tool domain: create, inspect, update, stop and collect agent tasks.
//!
//! Moved out of `lib.rs` as one domain (DEV-STRUCT-01). Bodies are verbatim; the
//! only change is the visibility the crate root needs.
//!
//! The registry these handlers operate on is process-global and lives in
//! [`crate::registries`] rather than here, because the worker handlers read it
//! too.

use serde::Deserialize;
use serde_json::{json, Value};

use runtime::TaskPacket;

use super::to_pretty_json;
use crate::registries::global_task_registry;

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn run_task_create(input: TaskCreateInput) -> Result<String, String> {
    let registry = global_task_registry();
    let task = registry.create(&input.prompt, input.description.as_deref());
    to_pretty_json(json!({
        "task_id": task.task_id,
        "status": task.status,
        "prompt": task.prompt,
        "description": task.description,
        "task_packet": task.task_packet,
        "created_at": task.created_at
    }))
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn run_task_packet(input: TaskPacket) -> Result<String, String> {
    let registry = global_task_registry();
    let task = registry.create_from_packet(input).map_err(|error| error.to_string())?;

    to_pretty_json(json!({
        "task_id": task.task_id,
        "status": task.status,
        "prompt": task.prompt,
        "description": task.description,
        "task_packet": task.task_packet,
        "created_at": task.created_at
    }))
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn run_task_get(input: TaskIdInput) -> Result<String, String> {
    let registry = global_task_registry();
    match registry.get(&input.task_id) {
        Some(task) => to_pretty_json(json!({
            "task_id": task.task_id,
            "status": task.status,
            "prompt": task.prompt,
            "description": task.description,
            "task_packet": task.task_packet,
            "created_at": task.created_at,
            "updated_at": task.updated_at,
            "messages": task.messages,
            "team_id": task.team_id
        })),
        None => Err(format!("task not found: {}", input.task_id)),
    }
}

pub(crate) fn run_task_list(_input: Value) -> Result<String, String> {
    let registry = global_task_registry();
    let tasks: Vec<_> = registry
        .list(None)
        .into_iter()
        .map(|t| {
            json!({
                "task_id": t.task_id,
                "status": t.status,
                "prompt": t.prompt,
                "description": t.description,
                "task_packet": t.task_packet,
                "created_at": t.created_at,
                "updated_at": t.updated_at,
                "team_id": t.team_id
            })
        })
        .collect();
    to_pretty_json(json!({
        "tasks": tasks,
        "count": tasks.len()
    }))
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn run_task_stop(input: TaskIdInput) -> Result<String, String> {
    let registry = global_task_registry();
    match registry.stop(&input.task_id) {
        Ok(task) => to_pretty_json(json!({
            "task_id": task.task_id,
            "status": task.status,
            "message": "Task stopped"
        })),
        Err(e) => Err(e),
    }
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn run_task_update(input: TaskUpdateInput) -> Result<String, String> {
    let registry = global_task_registry();
    match registry.update(&input.task_id, &input.message) {
        Ok(task) => to_pretty_json(json!({
            "task_id": task.task_id,
            "status": task.status,
            "message_count": task.messages.len(),
            "last_message": input.message
        })),
        Err(e) => Err(e),
    }
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn run_task_output(input: TaskIdInput) -> Result<String, String> {
    let registry = global_task_registry();
    match registry.output(&input.task_id) {
        Ok(output) => to_pretty_json(json!({
            "task_id": input.task_id,
            "output": output,
            "has_output": !output.is_empty()
        })),
        Err(e) => Err(e),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct TaskCreateInput {
    prompt: String,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TaskIdInput {
    task_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TaskUpdateInput {
    task_id: String,
    message: String,
}
