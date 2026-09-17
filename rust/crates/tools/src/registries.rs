//! Process-global registries shared by the tool domains.
//!
//! These six are `OnceLock` singletons: one per process, holding state that
//! outlives a single tool invocation (tasks, workers, teams, cron entries, the
//! language server and the MCP tool list). They are not the property of any one
//! domain - the task handlers and the worker handlers both read the task
//! registry - so they live here rather than in `lib.rs` or in a domain module.
//!
//! `GlobalToolRegistry` deliberately does **not** live here: it is the tool
//! catalogue and its dispatch, which the crate root owns, not process state.
//!
//! Extracted from `lib.rs` as the shared core for the domain split
//! (DEV-STRUCT-01); the function bodies are verbatim.

use runtime::{
    lsp_client::LspRegistry,
    mcp_tool_bridge::McpToolRegistry,
    task_registry::TaskRegistry,
    team_cron_registry::{CronRegistry, TeamRegistry},
    worker_boot::WorkerRegistry,
};

/// Language-server registry, shared across tool invocations in a session.
pub(crate) fn global_lsp_registry() -> &'static LspRegistry {
    use std::sync::OnceLock;
    static REGISTRY: OnceLock<LspRegistry> = OnceLock::new();
    REGISTRY.get_or_init(LspRegistry::new)
}

/// Registry of tools discovered from connected MCP servers.
pub(crate) fn global_mcp_registry() -> &'static McpToolRegistry {
    use std::sync::OnceLock;
    static REGISTRY: OnceLock<McpToolRegistry> = OnceLock::new();
    REGISTRY.get_or_init(McpToolRegistry::new)
}

/// Team registry, shared across tool invocations in a session.
pub(crate) fn global_team_registry() -> &'static TeamRegistry {
    use std::sync::OnceLock;
    static REGISTRY: OnceLock<TeamRegistry> = OnceLock::new();
    REGISTRY.get_or_init(TeamRegistry::new)
}

/// Cron registry holding the scheduled entries for this process.
pub(crate) fn global_cron_registry() -> &'static CronRegistry {
    use std::sync::OnceLock;
    static REGISTRY: OnceLock<CronRegistry> = OnceLock::new();
    REGISTRY.get_or_init(CronRegistry::new)
}

/// Task registry: the agent tasks this process has created.
pub(crate) fn global_task_registry() -> &'static TaskRegistry {
    use std::sync::OnceLock;
    static REGISTRY: OnceLock<TaskRegistry> = OnceLock::new();
    REGISTRY.get_or_init(TaskRegistry::new)
}

/// Worker registry for spawned workers and their trust state.
pub(crate) fn global_worker_registry() -> &'static WorkerRegistry {
    use std::sync::OnceLock;
    static REGISTRY: OnceLock<WorkerRegistry> = OnceLock::new();
    REGISTRY.get_or_init(WorkerRegistry::new)
}
