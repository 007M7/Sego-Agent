//! Tests for this crate (moved out of `lib.rs`).
//!
//! A move, not a rewrite: the module keeps its name and its imports, and
//! `super` is still the crate root. rustfmt normalises code indentation and
//! never string contents, so the body below is copied verbatim.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use super::{
    agent_permission_policy, allowed_tools_for_subagent, classify_lane_failure,
    execute_agent_with_spawn, execute_tool, execute_web_fetch_with, final_assistant_text,
    mvp_tool_specs, normalize_fetch_url_with, permission_mode_from_plugin,
    persist_agent_terminal_state, push_output_block, run_task_packet, AgentInput, AgentJob,
    GlobalToolRegistry, LaneEventName, LaneFailureClass, SubagentToolExecutor, WebFetchInput,
    UNTRUSTED_FETCH_NOTICE, UNTRUSTED_FETCH_TRUST,
};
use api::OutputContentBlock;
use runtime::{
    permission_enforcer::PermissionEnforcer, ApiRequest, AssistantEvent, ConversationRuntime,
    PermissionMode, PermissionPolicy, RuntimeError, Session, TaskPacket, ToolExecutor,
};
use serde_json::json;

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Restores the working directory when it leaves scope.
///
/// The file operations treat the working directory as the workspace root,
/// so a test that fails before restoring it would otherwise leak the change
/// into every later test in this binary.
struct CwdGuard {
    previous: PathBuf,
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.previous);
    }
}

fn enter_workspace(workspace: &Path) -> CwdGuard {
    let previous = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(workspace).expect("enter workspace");
    CwdGuard { previous }
}

fn temp_path(name: &str) -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    std::env::temp_dir().join(format!("clawd-tools-{unique}-{name}"))
}

fn shell_stdout_command(text: &str) -> String {
    if cfg!(windows) {
        format!("echo {text}")
    } else {
        format!("printf '{}'", text.replace('\'', "'\\''"))
    }
}

fn shell_stderr_exit_command(text: &str, code: i32) -> String {
    if cfg!(windows) {
        format!("echo {text} 1>&2 & exit /B {code}")
    } else {
        format!("printf '{}' >&2; exit {code}", text.replace('\'', "'\\''"))
    }
}

fn shell_sleep_command(seconds: u64) -> String {
    if cfg!(windows) {
        format!("ping -n {} 127.0.0.1 >NUL", seconds + 1)
    } else {
        format!("sleep {seconds}")
    }
}

fn run_git(cwd: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .unwrap_or_else(|error| panic!("git {} failed: {error}", args.join(" ")));
    assert!(status.success(), "git {} exited with {status}", args.join(" "));
}

fn init_git_repo(path: &Path) {
    std::fs::create_dir_all(path).expect("create repo");
    run_git(path, &["init", "--quiet", "-b", "main"]);
    run_git(path, &["config", "user.email", "tests@example.com"]);
    run_git(path, &["config", "user.name", "Tools Tests"]);
    std::fs::write(path.join("README.md"), "initial\n").expect("write readme");
    run_git(path, &["add", "README.md"]);
    run_git(path, &["commit", "-m", "initial commit", "--quiet"]);
}

fn commit_file(path: &Path, file: &str, contents: &str, message: &str) {
    std::fs::write(path.join(file), contents).expect("write file");
    run_git(path, &["add", file]);
    run_git(path, &["commit", "-m", message, "--quiet"]);
}

fn permission_policy_for_mode(mode: PermissionMode) -> PermissionPolicy {
    mvp_tool_specs().into_iter().fold(PermissionPolicy::new(mode), |policy, spec| {
        policy.with_tool_requirement(spec.name, spec.required_permission)
    })
}

#[test]
fn exposes_mvp_tools() {
    let names = mvp_tool_specs().into_iter().map(|spec| spec.name).collect::<Vec<_>>();
    assert!(names.contains(&"bash"));
    assert!(names.contains(&"read_file"));
    assert!(names.contains(&"WebFetch"));
    assert!(names.contains(&"WebSearch"));
    assert!(names.contains(&"TodoWrite"));
    assert!(names.contains(&"Skill"));
    assert!(names.contains(&"Agent"));
    assert!(names.contains(&"ToolSearch"));
    assert!(names.contains(&"NotebookEdit"));
    assert!(names.contains(&"Sleep"));
    assert!(names.contains(&"SendUserMessage"));
    assert!(names.contains(&"Config"));
    assert!(names.contains(&"EnterPlanMode"));
    assert!(names.contains(&"ExitPlanMode"));
    assert!(names.contains(&"StructuredOutput"));
    assert!(names.contains(&"REPL"));
    assert!(names.contains(&"PowerShell"));
    assert!(names.contains(&"WorkerCreate"));
    assert!(names.contains(&"WorkerObserve"));
    assert!(names.contains(&"WorkerAwaitReady"));
    assert!(names.contains(&"WorkerSendPrompt"));
}

#[test]
fn rejects_unknown_tool_names() {
    let error = execute_tool("nope", &json!({})).expect_err("tool should be rejected");
    assert!(error.contains("unsupported tool"));
}

#[test]
fn worker_tools_gate_prompt_delivery_until_ready_and_support_auto_trust() {
    let created = execute_tool(
        "WorkerCreate",
        &json!({
            "cwd": "/tmp/worktree/repo",
            "trusted_roots": ["/tmp/worktree"]
        }),
    )
    .expect("WorkerCreate should succeed");
    let created_output: serde_json::Value = serde_json::from_str(&created).expect("json");
    let worker_id = created_output["worker_id"].as_str().expect("worker id").to_string();
    assert_eq!(created_output["status"], "spawning");
    assert_eq!(created_output["trust_auto_resolve"], true);

    let gated = execute_tool(
        "WorkerSendPrompt",
        &json!({
            "worker_id": worker_id,
            "prompt": "ship the change"
        }),
    )
    .expect_err("prompt delivery before ready should fail");
    assert!(gated.contains("not ready for prompt delivery"));

    let observed = execute_tool(
        "WorkerObserve",
        &json!({
            "worker_id": created_output["worker_id"],
            "screen_text": "Do you trust the files in this folder?\n1. Yes, proceed\n2. No"
        }),
    )
    .expect("WorkerObserve should auto-resolve trust");
    let observed_output: serde_json::Value = serde_json::from_str(&observed).expect("json");
    assert_eq!(observed_output["status"], "spawning");
    assert_eq!(observed_output["trust_gate_cleared"], true);
    assert_eq!(observed_output["events"][1]["payload"]["type"], "trust_prompt");
    assert_eq!(observed_output["events"][2]["payload"]["resolution"], "auto_allowlisted");

    let ready = execute_tool(
        "WorkerObserve",
        &json!({
            "worker_id": created_output["worker_id"],
            "screen_text": "Ready for your input\n>"
        }),
    )
    .expect("WorkerObserve should mark worker ready");
    let ready_output: serde_json::Value = serde_json::from_str(&ready).expect("json");
    assert_eq!(ready_output["status"], "ready_for_prompt");

    let await_ready = execute_tool(
        "WorkerAwaitReady",
        &json!({
            "worker_id": created_output["worker_id"]
        }),
    )
    .expect("WorkerAwaitReady should succeed");
    let await_ready_output: serde_json::Value = serde_json::from_str(&await_ready).expect("json");
    assert_eq!(await_ready_output["ready"], true);

    let accepted = execute_tool(
        "WorkerSendPrompt",
        &json!({
            "worker_id": created_output["worker_id"],
            "prompt": "ship the change"
        }),
    )
    .expect("WorkerSendPrompt should succeed after ready");
    let accepted_output: serde_json::Value = serde_json::from_str(&accepted).expect("json");
    assert_eq!(accepted_output["status"], "running");
    assert_eq!(accepted_output["prompt_delivery_attempts"], 1);
    assert_eq!(accepted_output["prompt_in_flight"], true);
}

#[test]
fn worker_tools_detect_misdelivery_and_arm_prompt_replay() {
    let created = execute_tool(
        "WorkerCreate",
        &json!({
            "cwd": "/tmp/repo/worker-misdelivery"
        }),
    )
    .expect("WorkerCreate should succeed");
    let created_output: serde_json::Value = serde_json::from_str(&created).expect("json");
    let worker_id = created_output["worker_id"].as_str().expect("worker id").to_string();

    execute_tool(
        "WorkerObserve",
        &json!({
            "worker_id": worker_id,
            "screen_text": "Ready for input\n>"
        }),
    )
    .expect("worker should become ready");

    execute_tool(
        "WorkerSendPrompt",
        &json!({
            "worker_id": worker_id,
            "prompt": "Investigate flaky boot"
        }),
    )
    .expect("prompt send should succeed");

    let recovered = execute_tool(
        "WorkerObserve",
        &json!({
            "worker_id": worker_id,
            "screen_text": "% Investigate flaky boot\nzsh: command not found: Investigate"
        }),
    )
    .expect("misdelivery observe should succeed");
    let recovered_output: serde_json::Value = serde_json::from_str(&recovered).expect("json");
    assert_eq!(recovered_output["status"], "ready_for_prompt");
    assert_eq!(recovered_output["last_error"]["kind"], "prompt_delivery");
    assert_eq!(recovered_output["replay_prompt"], "Investigate flaky boot");
    assert_eq!(recovered_output["events"][3]["payload"]["observed_target"], "shell");
    assert_eq!(recovered_output["events"][4]["payload"]["recovery_armed"], true);

    let replayed = execute_tool(
        "WorkerSendPrompt",
        &json!({
            "worker_id": worker_id
        }),
    )
    .expect("WorkerSendPrompt should replay recovered prompt");
    let replayed_output: serde_json::Value = serde_json::from_str(&replayed).expect("json");
    assert_eq!(replayed_output["status"], "running");
    assert_eq!(replayed_output["prompt_delivery_attempts"], 2);
    assert_eq!(replayed_output["prompt_in_flight"], true);
}

#[test]
fn global_tool_registry_denies_blocked_tool_before_dispatch() {
    // given
    let policy = permission_policy_for_mode(PermissionMode::ReadOnly);
    let registry = GlobalToolRegistry::builtin().with_enforcer(PermissionEnforcer::new(policy));

    // when
    let error = registry
        .execute(
            "write_file",
            &json!({
                "path": "blocked.txt",
                "content": "blocked"
            }),
        )
        .expect_err("write tool should be denied before dispatch");

    // then
    assert!(error.contains("requires workspace-write permission"));
}

#[test]
fn subagent_tool_executor_denies_blocked_tool_before_dispatch() {
    // given
    let policy = permission_policy_for_mode(PermissionMode::ReadOnly);
    let mut executor = SubagentToolExecutor::new(BTreeSet::from([String::from("write_file")]))
        .with_enforcer(PermissionEnforcer::new(policy));

    // when
    let error = executor
        .execute(
            "write_file",
            &json!({
                "path": "blocked.txt",
                "content": "blocked"
            })
            .to_string(),
        )
        .expect_err("subagent write tool should be denied before dispatch");

    // then
    assert!(error.to_string().contains("requires workspace-write permission"));
}

#[test]
fn permission_mode_from_plugin_rejects_invalid_inputs() {
    let unknown_permission =
        permission_mode_from_plugin("admin").expect_err("unknown plugin permission should fail");
    assert!(unknown_permission.contains("unsupported plugin permission: admin"));

    let empty_permission =
        permission_mode_from_plugin("").expect_err("empty plugin permission should fail");
    assert!(empty_permission.contains("unsupported plugin permission: "));
}

#[test]
fn runtime_tools_extend_registry_definitions_permissions_and_search() {
    let registry = GlobalToolRegistry::builtin()
        .with_runtime_tools(vec![super::RuntimeToolDefinition {
            name: "mcp__demo__echo".to_string(),
            description: Some("Echo text from the demo MCP server".to_string()),
            input_schema: json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "additionalProperties": false
            }),
            required_permission: runtime::PermissionMode::ReadOnly,
        }])
        .expect("runtime tools should register");

    let allowed = registry
        .normalize_allowed_tools(&["mcp__demo__echo".to_string()])
        .expect("runtime tool should be allow-listable")
        .expect("allow-list should be populated");
    assert!(allowed.contains("mcp__demo__echo"));

    let definitions = registry.definitions(Some(&allowed));
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].name, "mcp__demo__echo");

    let permissions =
        registry.permission_specs(Some(&allowed)).expect("runtime tool permissions should resolve");
    assert_eq!(
        permissions,
        vec![("mcp__demo__echo".to_string(), runtime::PermissionMode::ReadOnly)]
    );

    let search = registry.search(
        "demo echo",
        5,
        Some(vec!["pending-server".to_string()]),
        Some(runtime::McpDegradedReport::new(
            vec!["demo".to_string()],
            vec![runtime::McpFailedServer {
                server_name: "pending-server".to_string(),
                phase: runtime::McpLifecyclePhase::ToolDiscovery,
                error: runtime::McpErrorSurface::new(
                    runtime::McpLifecyclePhase::ToolDiscovery,
                    Some("pending-server".to_string()),
                    "tool discovery failed",
                    BTreeMap::new(),
                    true,
                ),
            }],
            vec!["mcp__demo__echo".to_string()],
            vec!["mcp__demo__echo".to_string()],
        )),
    );
    let output = serde_json::to_value(search).expect("search output should serialize");
    assert_eq!(output["matches"][0], "mcp__demo__echo");
    assert_eq!(output["pending_mcp_servers"][0], "pending-server");
    assert_eq!(output["mcp_degraded"]["failed_servers"][0]["phase"], "tool_discovery");
}

#[test]
fn web_fetch_returns_prompt_aware_summary() {
    let server = TestServer::spawn(Arc::new(|request_line: &str| {
        assert!(request_line.starts_with("GET /page "));
        HttpResponse::html(
                200,
                "OK",
                "<html><head><title>Ignored</title></head><body><h1>Test Page</h1><p>Hello <b>world</b> from local server.</p></body></html>",
            )
    }));

    // The public path refuses loopback, so tests opt in explicitly instead
    // of weakening the production policy.
    let input = WebFetchInput {
        url: format!("http://{}/page", server.addr()),
        prompt: "Summarize this page".to_string(),
    };
    let fetched = execute_web_fetch_with(&input, true).expect("WebFetch should succeed");
    let output = serde_json::to_value(&fetched).expect("output should serialize");
    assert_eq!(output["code"], 200);
    assert_eq!(output["trust"], UNTRUSTED_FETCH_TRUST);
    assert_eq!(output["truncated"], false);
    assert_eq!(output["redirects"], 0);
    let summary = output["result"].as_str().expect("result string");
    assert!(summary.contains(UNTRUSTED_FETCH_NOTICE), "fetched text must be marked untrusted");
    assert!(summary.contains("Fetched"));
    assert!(summary.contains("Test Page"));
    assert!(summary.contains("Hello world from local server"));

    let titled_input = WebFetchInput {
        url: format!("http://{}/page", server.addr()),
        prompt: "What is the page title?".to_string(),
    };
    let titled = execute_web_fetch_with(&titled_input, true).expect("title query");
    let titled_output = serde_json::to_value(&titled).expect("serialize");
    let titled_summary = titled_output["result"].as_str().expect("result string");
    assert!(titled_summary.contains("Title: Ignored"));
}

#[test]
fn web_fetch_supports_plain_text_and_rejects_invalid_url() {
    let server = TestServer::spawn(Arc::new(|request_line: &str| {
        assert!(request_line.starts_with("GET /plain "));
        HttpResponse::text(200, "OK", "plain text response")
    }));

    let input = WebFetchInput {
        url: format!("http://{}/plain", server.addr()),
        prompt: "Show me the content".to_string(),
    };
    let fetched = execute_web_fetch_with(&input, true).expect("text content should succeed");
    let output = serde_json::to_value(&fetched).expect("serialize");
    assert_eq!(output["url"], format!("http://{}/plain", server.addr()));
    assert!(output["result"].as_str().expect("result").contains("plain text response"));

    let error = execute_tool(
        "WebFetch",
        &json!({
            "url": "not a url",
            "prompt": "Summarize"
        }),
    )
    .expect_err("invalid URL should fail");
    assert!(error.contains("relative URL without a base") || error.contains("invalid"));
}

#[test]
fn web_fetch_refuses_loopback_private_and_metadata_targets() {
    let cases = [
        ("http://127.0.0.1:9/x", "loopback"),
        ("http://localhost/x", "loopback"),
        ("http://[::1]/x", "loopback"),
        ("http://127.9.9.9/x", "loopback"),
        ("http://169.254.169.254/latest/meta-data/", "link-local"),
        ("http://10.1.2.3/x", "private"),
        ("http://192.168.0.1/x", "private"),
        ("http://172.16.5.5/x", "private"),
        ("http://100.64.0.1/x", "carrier-grade NAT"),
        ("http://metadata.google.internal/x", "cloud metadata"),
        ("http://[fd00::1]/x", "unique-local"),
        ("http://[fe80::1]/x", "link-local"),
        ("http://0.0.0.0/x", "reserved"),
    ];
    for (url, expected) in cases {
        let error = execute_tool("WebFetch", &json!({ "url": url, "prompt": "x" }))
            .expect_err(&format!("{url} must be refused"));
        assert!(error.contains(expected), "{url} should mention {expected}, got: {error}");
    }
    // Non-http(s) schemes are refused before any request is made.
    let scheme_error =
        execute_tool("WebFetch", &json!({ "url": "ftp://example.com/x", "prompt": "x" }))
            .expect_err("ftp must be refused");
    assert!(scheme_error.contains("unsupported URL scheme"), "{scheme_error}");
}

#[test]
fn normalize_fetch_url_blocks_internal_targets_and_upgrades_public_http() {
    for blocked in [
        "http://127.0.0.1:8080/x",
        "http://10.0.0.1/x",
        "http://169.254.169.254/x",
        "http://[::1]/x",
        "http://localhost/x",
        "http://metadata/x",
        "http://[fd00::1]/x",
        "http://100.64.0.1/x",
    ] {
        let error = normalize_fetch_url_with(blocked, false)
            .expect_err(&format!("{blocked} must be blocked"));
        assert!(!error.is_empty(), "{blocked} must produce a reason");
    }
    // Explicit opt-in is what the local-mock tests use.
    assert_eq!(
        normalize_fetch_url_with("http://127.0.0.1:8080/x", true).expect("loopback on opt-in"),
        "http://127.0.0.1:8080/x"
    );
    // Public hosts may not be downgraded: plain http is upgraded to https.
    assert_eq!(
        normalize_fetch_url_with("http://example.com/x", false).expect("public host"),
        "https://example.com/x"
    );
    // Schemes other than http(s) are refused before any request.
    assert!(normalize_fetch_url_with("ftp://example.com/x", false).is_err());
    assert!(normalize_fetch_url_with("file:///etc/passwd", false).is_err());
}

#[test]
fn web_fetch_revalidates_every_hop_instead_of_following_blindly() {
    // A page that redirects to a cloud metadata address. With reqwest's own
    // redirect policy this was followed, because only the first URL was ever
    // checked; the hop policy is what has to refuse it.
    let redirector = TestServer::spawn(Arc::new(|request_line: &str| {
        assert!(request_line.starts_with("GET /jump "), "{request_line}");
        HttpResponse::redirect("http://169.254.169.254/latest/meta-data/")
    }));

    let input = WebFetchInput {
        url: format!("http://{}/jump", redirector.addr()),
        prompt: "Summarize".to_string(),
    };
    let error = execute_web_fetch_with(&input, true)
        .expect_err("a redirect to a metadata address must not be followed");
    assert!(error.contains("link-local"), "{error}");
    assert!(error.contains("169.254.169.254"), "{error}");
}

#[test]
fn web_fetch_follows_a_permitted_redirect_and_counts_it() {
    let target = TestServer::spawn(Arc::new(|request_line: &str| {
        assert!(request_line.starts_with("GET /final "), "{request_line}");
        HttpResponse::text(200, "OK", "arrived")
    }));
    let location = format!("http://{}/final", target.addr());
    let redirector = TestServer::spawn(Arc::new(move |_| HttpResponse::redirect(&location)));

    let input = WebFetchInput {
        url: format!("http://{}/start", redirector.addr()),
        prompt: "Show me the content".to_string(),
    };
    let fetched =
        execute_web_fetch_with(&input, true).expect("a permitted redirect should be followed");
    let output = serde_json::to_value(&fetched).expect("serialize");
    assert_eq!(output["redirects"], 1);
    assert!(output["result"].as_str().expect("result").contains("arrived"));
}

#[test]
fn web_fetch_stops_after_the_redirect_cap() {
    // A server that redirects to itself: the hop cap has to stop it, not the
    // client's own limit or a stack overflow.
    let shared: Arc<Mutex<Option<SocketAddr>>> = Arc::new(Mutex::new(None));
    let for_handler = Arc::clone(&shared);
    let server = TestServer::spawn(Arc::new(move |_| {
        let target = for_handler.lock().expect("lock").expect("addr is set below");
        HttpResponse::redirect(&format!("http://{target}/loop"))
    }));
    *shared.lock().expect("lock") = Some(server.addr());

    let input =
        WebFetchInput { url: format!("http://{}/loop", server.addr()), prompt: "x".to_string() };
    let error = execute_web_fetch_with(&input, true).expect_err("the hop cap must stop the loop");
    assert!(error.contains("redirects"), "{error}");
}

#[test]
fn web_search_extracts_and_filters_results() {
    let server = TestServer::spawn(Arc::new(|request_line: &str| {
        assert!(request_line.contains("GET /search?q=rust+web+search "));
        HttpResponse::html(
            200,
            "OK",
            r#"
                <html><body>
                  <a class="result__a" href="https://docs.rs/reqwest">Reqwest docs</a>
                  <a class="result__a" href="https://example.com/blocked">Blocked result</a>
                </body></html>
                "#,
        )
    }));

    std::env::set_var("CLAWD_WEB_SEARCH_BASE_URL", format!("http://{}/search", server.addr()));
    let result = execute_tool(
        "WebSearch",
        &json!({
            "query": "rust web search",
            "allowed_domains": ["https://DOCS.rs/"],
            "blocked_domains": ["HTTPS://EXAMPLE.COM"]
        }),
    )
    .expect("WebSearch should succeed");
    std::env::remove_var("CLAWD_WEB_SEARCH_BASE_URL");

    let output: serde_json::Value = serde_json::from_str(&result).expect("valid json");
    assert_eq!(output["query"], "rust web search");
    let results = output["results"].as_array().expect("results array");
    let search_result = results
        .iter()
        .find(|item| item.get("content").is_some())
        .expect("search result block present");
    let content = search_result["content"].as_array().expect("content array");
    assert_eq!(content.len(), 1);
    assert_eq!(content[0]["title"], "Reqwest docs");
    assert_eq!(content[0]["url"], "https://docs.rs/reqwest");
}

#[test]
fn web_search_handles_generic_links_and_invalid_base_url() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let server = TestServer::spawn(Arc::new(|request_line: &str| {
        assert!(request_line.contains("GET /fallback?q=generic+links "));
        HttpResponse::html(
            200,
            "OK",
            r#"
                <html><body>
                  <a href="https://example.com/one">Example One</a>
                  <a href="https://example.com/one">Duplicate Example One</a>
                  <a href="https://docs.rs/tokio">Tokio Docs</a>
                </body></html>
                "#,
        )
    }));

    std::env::set_var("CLAWD_WEB_SEARCH_BASE_URL", format!("http://{}/fallback", server.addr()));
    let result = execute_tool(
        "WebSearch",
        &json!({
            "query": "generic links"
        }),
    )
    .expect("WebSearch fallback parsing should succeed");
    std::env::remove_var("CLAWD_WEB_SEARCH_BASE_URL");

    let output: serde_json::Value = serde_json::from_str(&result).expect("valid json");
    let results = output["results"].as_array().expect("results array");
    let search_result = results
        .iter()
        .find(|item| item.get("content").is_some())
        .expect("search result block present");
    let content = search_result["content"].as_array().expect("content array");
    assert_eq!(content.len(), 2);
    assert_eq!(content[0]["url"], "https://example.com/one");
    assert_eq!(content[1]["url"], "https://docs.rs/tokio");

    std::env::set_var("CLAWD_WEB_SEARCH_BASE_URL", "://bad-base-url");
    let error = execute_tool("WebSearch", &json!({ "query": "generic links" }))
        .expect_err("invalid base URL should fail");
    std::env::remove_var("CLAWD_WEB_SEARCH_BASE_URL");
    assert!(error.contains("relative URL without a base") || error.contains("empty host"));
}

#[test]
fn pending_tools_preserve_multiple_streaming_tool_calls_by_index() {
    let mut events = Vec::new();
    let mut pending_tools = BTreeMap::new();

    let mut pending_thinking: std::collections::BTreeMap<u32, (String, Option<String>)> =
        std::collections::BTreeMap::new();
    push_output_block(
        OutputContentBlock::ToolUse {
            id: "tool-1".to_string(),
            name: "read_file".to_string(),
            input: json!({}),
        },
        1,
        &mut events,
        &mut pending_tools,
        &mut pending_thinking,
        true,
    );
    push_output_block(
        OutputContentBlock::ToolUse {
            id: "tool-2".to_string(),
            name: "grep_search".to_string(),
            input: json!({}),
        },
        2,
        &mut events,
        &mut pending_tools,
        &mut pending_thinking,
        true,
    );

    pending_tools.get_mut(&1).expect("first tool pending").2.push_str("{\"path\":\"src/main.rs\"}");
    pending_tools.get_mut(&2).expect("second tool pending").2.push_str("{\"pattern\":\"TODO\"}");

    assert_eq!(
        pending_tools.remove(&1),
        Some((
            "tool-1".to_string(),
            "read_file".to_string(),
            "{\"path\":\"src/main.rs\"}".to_string(),
        ))
    );
    assert_eq!(
        pending_tools.remove(&2),
        Some((
            "tool-2".to_string(),
            "grep_search".to_string(),
            "{\"pattern\":\"TODO\"}".to_string(),
        ))
    );
}

#[test]
fn todo_write_persists_and_returns_previous_state() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let path = temp_path("todos.json");
    std::env::set_var("CLAWD_TODO_STORE", &path);

    let first = execute_tool(
        "TodoWrite",
        &json!({
            "todos": [
                {"content": "Add tool", "activeForm": "Adding tool", "status": "in_progress"},
                {"content": "Run tests", "activeForm": "Running tests", "status": "pending"}
            ]
        }),
    )
    .expect("TodoWrite should succeed");
    let first_output: serde_json::Value = serde_json::from_str(&first).expect("valid json");
    assert_eq!(first_output["oldTodos"].as_array().expect("array").len(), 0);

    let second = execute_tool(
        "TodoWrite",
        &json!({
            "todos": [
                {"content": "Add tool", "activeForm": "Adding tool", "status": "completed"},
                {"content": "Run tests", "activeForm": "Running tests", "status": "completed"},
                {"content": "Verify", "activeForm": "Verifying", "status": "completed"}
            ]
        }),
    )
    .expect("TodoWrite should succeed");
    std::env::remove_var("CLAWD_TODO_STORE");
    let _ = std::fs::remove_file(path);

    let second_output: serde_json::Value = serde_json::from_str(&second).expect("valid json");
    assert_eq!(second_output["oldTodos"].as_array().expect("array").len(), 2);
    assert_eq!(second_output["newTodos"].as_array().expect("array").len(), 3);
    assert!(second_output["verificationNudgeNeeded"].is_null());
}

#[test]
fn todo_write_rejects_invalid_payloads_and_sets_verification_nudge() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let path = temp_path("todos-errors.json");
    std::env::set_var("CLAWD_TODO_STORE", &path);

    let empty =
        execute_tool("TodoWrite", &json!({ "todos": [] })).expect_err("empty todos should fail");
    assert!(empty.contains("todos must not be empty"));

    // Multiple in_progress items are now allowed for parallel workflows
    let _multi_active = execute_tool(
        "TodoWrite",
        &json!({
            "todos": [
                {"content": "One", "activeForm": "Doing one", "status": "in_progress"},
                {"content": "Two", "activeForm": "Doing two", "status": "in_progress"}
            ]
        }),
    )
    .expect("multiple in-progress todos should succeed");

    let blank_content = execute_tool(
        "TodoWrite",
        &json!({
            "todos": [
                {"content": "   ", "activeForm": "Doing it", "status": "pending"}
            ]
        }),
    )
    .expect_err("blank content should fail");
    assert!(blank_content.contains("todo content must not be empty"));

    let nudge = execute_tool(
        "TodoWrite",
        &json!({
            "todos": [
                {"content": "Write tests", "activeForm": "Writing tests", "status": "completed"},
                {"content": "Fix errors", "activeForm": "Fixing errors", "status": "completed"},
                {"content": "Ship branch", "activeForm": "Shipping branch", "status": "completed"}
            ]
        }),
    )
    .expect("completed todos should succeed");
    std::env::remove_var("CLAWD_TODO_STORE");
    let _ = fs::remove_file(path);

    let output: serde_json::Value = serde_json::from_str(&nudge).expect("valid json");
    assert_eq!(output["verificationNudgeNeeded"], true);
}

#[test]
fn skill_loads_local_skill_prompt() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = temp_path("skills-home");
    let skill_dir = home.join(".agents").join("skills").join("help");
    fs::create_dir_all(&skill_dir).expect("skill dir should exist");
    fs::write(skill_dir.join("SKILL.md"), "# help\n\nGuide on using oh-my-codex plugin\n")
        .expect("skill file should exist");
    let original_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", &home);

    let result = execute_tool(
        "Skill",
        &json!({
            "skill": "help",
            "args": "overview"
        }),
    )
    .expect("Skill should succeed");

    let output: serde_json::Value = serde_json::from_str(&result).expect("valid json");
    assert_eq!(output["skill"], "help");
    assert!(output["path"].as_str().expect("path").replace('\\', "/").ends_with("/help/SKILL.md"));
    assert!(output["prompt"]
        .as_str()
        .expect("prompt")
        .contains("Guide on using oh-my-codex plugin"));

    let dollar_result = execute_tool(
        "Skill",
        &json!({
            "skill": "$help"
        }),
    )
    .expect("Skill should accept $skill invocation form");
    let dollar_output: serde_json::Value =
        serde_json::from_str(&dollar_result).expect("valid json");
    assert_eq!(dollar_output["skill"], "$help");
    assert!(dollar_output["path"]
        .as_str()
        .expect("path")
        .replace('\\', "/")
        .ends_with("/help/SKILL.md"));

    if let Some(home) = original_home {
        std::env::set_var("HOME", home);
    } else {
        std::env::remove_var("HOME");
    }
    fs::remove_dir_all(home).expect("temp home should clean up");
}

#[test]
fn tool_search_supports_keyword_and_select_queries() {
    let keyword = execute_tool("ToolSearch", &json!({"query": "web current", "max_results": 3}))
        .expect("ToolSearch should succeed");
    let keyword_output: serde_json::Value = serde_json::from_str(&keyword).expect("valid json");
    let matches = keyword_output["matches"].as_array().expect("matches");
    assert!(matches.iter().any(|value| value == "WebSearch"));

    let selected = execute_tool("ToolSearch", &json!({"query": "select:Agent,Skill"}))
        .expect("ToolSearch should succeed");
    let selected_output: serde_json::Value = serde_json::from_str(&selected).expect("valid json");
    assert_eq!(selected_output["matches"][0], "Agent");
    assert_eq!(selected_output["matches"][1], "Skill");

    let aliased = execute_tool("ToolSearch", &json!({"query": "AgentTool"}))
        .expect("ToolSearch should support tool aliases");
    let aliased_output: serde_json::Value = serde_json::from_str(&aliased).expect("valid json");
    assert_eq!(aliased_output["matches"][0], "Agent");
    assert_eq!(aliased_output["normalized_query"], "agent");

    let selected_with_alias =
        execute_tool("ToolSearch", &json!({"query": "select:AgentTool,Skill"}))
            .expect("ToolSearch alias select should succeed");
    let selected_with_alias_output: serde_json::Value =
        serde_json::from_str(&selected_with_alias).expect("valid json");
    assert_eq!(selected_with_alias_output["matches"][0], "Agent");
    assert_eq!(selected_with_alias_output["matches"][1], "Skill");
}

#[test]
fn agent_persists_handoff_metadata() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = temp_path("agent-store");
    std::env::set_var("CLAWD_AGENT_STORE", &dir);
    let captured = Arc::new(Mutex::new(None::<AgentJob>));
    let captured_for_spawn = Arc::clone(&captured);

    let manifest = execute_agent_with_spawn(
        AgentInput {
            description: "Audit the branch".to_string(),
            prompt: "Check tests and outstanding work.".to_string(),
            subagent_type: Some("Explore".to_string()),
            name: Some("ship-audit".to_string()),
            model: None,
        },
        move |job| {
            *captured_for_spawn.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some(job);
            Ok(())
        },
    )
    .expect("Agent should succeed");
    std::env::remove_var("CLAWD_AGENT_STORE");

    assert_eq!(manifest.name, "ship-audit");
    assert_eq!(manifest.subagent_type.as_deref(), Some("Explore"));
    assert_eq!(manifest.status, "running");
    assert!(!manifest.created_at.is_empty());
    assert!(manifest.started_at.is_some());
    assert!(manifest.completed_at.is_none());
    let contents = std::fs::read_to_string(&manifest.output_file).expect("agent file exists");
    let manifest_contents =
        std::fs::read_to_string(&manifest.manifest_file).expect("manifest file exists");
    let manifest_json: serde_json::Value =
        serde_json::from_str(&manifest_contents).expect("manifest should be valid json");
    assert!(contents.contains("Audit the branch"));
    assert!(contents.contains("Check tests and outstanding work."));
    assert!(manifest_contents.contains("\"subagentType\": \"Explore\""));
    assert!(manifest_contents.contains("\"status\": \"running\""));
    assert_eq!(manifest_json["laneEvents"][0]["event"], "lane.started");
    assert_eq!(manifest_json["laneEvents"][0]["status"], "running");
    assert!(manifest_json["currentBlocker"].is_null());
    let captured_job = captured
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .expect("spawn job should be captured");
    assert_eq!(captured_job.prompt, "Check tests and outstanding work.");
    assert!(captured_job.allowed_tools.contains("read_file"));
    assert!(!captured_job.allowed_tools.contains("Agent"));

    let normalized = execute_tool(
        "Agent",
        &json!({
            "description": "Verify the branch",
            "prompt": "Check tests.",
            "subagent_type": "explorer"
        }),
    )
    .expect("Agent should normalize built-in aliases");
    let normalized_output: serde_json::Value =
        serde_json::from_str(&normalized).expect("valid json");
    assert_eq!(normalized_output["subagentType"], "Explore");

    let named = execute_tool(
        "Agent",
        &json!({
            "description": "Review the branch",
            "prompt": "Inspect diff.",
            "name": "Ship Audit!!!"
        }),
    )
    .expect("Agent should normalize explicit names");
    let named_output: serde_json::Value = serde_json::from_str(&named).expect("valid json");
    assert_eq!(named_output["name"], "ship-audit");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn agent_fake_runner_can_persist_completion_and_failure() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = temp_path("agent-runner");
    std::env::set_var("CLAWD_AGENT_STORE", &dir);

    let completed = execute_agent_with_spawn(
        AgentInput {
            description: "Complete the task".to_string(),
            prompt: "Do the work".to_string(),
            subagent_type: Some("Explore".to_string()),
            name: Some("complete-task".to_string()),
            model: Some("claude-sonnet-4-6".to_string()),
        },
        |job| {
            persist_agent_terminal_state(
                &job.manifest,
                "completed",
                Some("Finished successfully"),
                None,
            )
        },
    )
    .expect("completed agent should succeed");

    let completed_manifest =
        std::fs::read_to_string(&completed.manifest_file).expect("completed manifest should exist");
    let completed_manifest_json: serde_json::Value =
        serde_json::from_str(&completed_manifest).expect("completed manifest json");
    let completed_output =
        std::fs::read_to_string(&completed.output_file).expect("completed output should exist");
    assert!(completed_manifest.contains("\"status\": \"completed\""));
    assert!(completed_output.contains("Finished successfully"));
    assert_eq!(completed_manifest_json["laneEvents"][0]["event"], "lane.started");
    assert_eq!(completed_manifest_json["laneEvents"][1]["event"], "lane.finished");
    assert!(completed_manifest_json["currentBlocker"].is_null());

    let failed = execute_agent_with_spawn(
        AgentInput {
            description: "Fail the task".to_string(),
            prompt: "Do the failing work".to_string(),
            subagent_type: Some("Verification".to_string()),
            name: Some("fail-task".to_string()),
            model: None,
        },
        |job| {
            persist_agent_terminal_state(
                &job.manifest,
                "failed",
                None,
                Some(String::from("tool failed: simulated failure")),
            )
        },
    )
    .expect("failed agent should still spawn");

    let failed_manifest =
        std::fs::read_to_string(&failed.manifest_file).expect("failed manifest should exist");
    let failed_manifest_json: serde_json::Value =
        serde_json::from_str(&failed_manifest).expect("failed manifest json");
    let failed_output =
        std::fs::read_to_string(&failed.output_file).expect("failed output should exist");
    assert!(failed_manifest.contains("\"status\": \"failed\""));
    assert!(failed_manifest.contains("simulated failure"));
    assert!(failed_output.contains("simulated failure"));
    assert!(failed_output.contains("failure_class: tool_runtime"));
    assert_eq!(failed_manifest_json["currentBlocker"]["failureClass"], "tool_runtime");
    assert_eq!(failed_manifest_json["laneEvents"][1]["event"], "lane.blocked");
    assert_eq!(failed_manifest_json["laneEvents"][2]["event"], "lane.failed");
    assert_eq!(failed_manifest_json["laneEvents"][2]["failureClass"], "tool_runtime");

    let spawn_error = execute_agent_with_spawn(
        AgentInput {
            description: "Spawn error task".to_string(),
            prompt: "Never starts".to_string(),
            subagent_type: None,
            name: Some("spawn-error".to_string()),
            model: None,
        },
        |_| Err(String::from("thread creation failed")),
    )
    .expect_err("spawn errors should surface");
    assert!(spawn_error.contains("failed to spawn sub-agent"));
    let spawn_error_manifest = std::fs::read_dir(&dir)
        .expect("agent dir should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .find_map(|path| {
            let contents = std::fs::read_to_string(&path).ok()?;
            contents.contains("\"name\": \"spawn-error\"").then_some(contents)
        })
        .expect("failed manifest should still be written");
    let spawn_error_manifest_json: serde_json::Value =
        serde_json::from_str(&spawn_error_manifest).expect("spawn error manifest json");
    assert!(spawn_error_manifest.contains("\"status\": \"failed\""));
    assert!(spawn_error_manifest.contains("thread creation failed"));
    assert_eq!(spawn_error_manifest_json["currentBlocker"]["failureClass"], "infra");

    std::env::remove_var("CLAWD_AGENT_STORE");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn lane_failure_taxonomy_normalizes_common_blockers() {
    let cases = [
        ("prompt delivery failed in tmux pane", LaneFailureClass::PromptDelivery),
        ("trust prompt is still blocking startup", LaneFailureClass::TrustGate),
        ("branch stale against main after divergence", LaneFailureClass::BranchDivergence),
        ("compile failed after cargo check", LaneFailureClass::Compile),
        ("targeted tests failed", LaneFailureClass::Test),
        ("plugin bootstrap failed", LaneFailureClass::PluginStartup),
        ("mcp handshake timed out", LaneFailureClass::McpHandshake),
        ("mcp startup failed before listing tools", LaneFailureClass::McpStartup),
        ("gateway routing rejected the request", LaneFailureClass::GatewayRouting),
        ("tool failed: denied tool execution from hook", LaneFailureClass::ToolRuntime),
        ("thread creation failed", LaneFailureClass::Infra),
    ];

    for (message, expected) in cases {
        assert_eq!(classify_lane_failure(message), expected, "{message}");
    }
}

#[test]
fn lane_event_schema_serializes_to_canonical_names() {
    let cases = [
        (LaneEventName::Started, "lane.started"),
        (LaneEventName::Ready, "lane.ready"),
        (LaneEventName::PromptMisdelivery, "lane.prompt_misdelivery"),
        (LaneEventName::Blocked, "lane.blocked"),
        (LaneEventName::Red, "lane.red"),
        (LaneEventName::Green, "lane.green"),
        (LaneEventName::CommitCreated, "lane.commit.created"),
        (LaneEventName::PrOpened, "lane.pr.opened"),
        (LaneEventName::MergeReady, "lane.merge.ready"),
        (LaneEventName::Finished, "lane.finished"),
        (LaneEventName::Failed, "lane.failed"),
        (LaneEventName::BranchStaleAgainstMain, "branch.stale_against_main"),
    ];

    for (event, expected) in cases {
        assert_eq!(serde_json::to_value(event).expect("serialize lane event"), json!(expected));
    }
}

#[test]
fn agent_tool_subset_mapping_is_expected() {
    let general = allowed_tools_for_subagent("general-purpose");
    assert!(general.contains("bash"));
    assert!(general.contains("write_file"));
    assert!(!general.contains("Agent"));

    let explore = allowed_tools_for_subagent("Explore");
    assert!(explore.contains("read_file"));
    assert!(explore.contains("grep_search"));
    assert!(!explore.contains("bash"));

    let plan = allowed_tools_for_subagent("Plan");
    assert!(plan.contains("TodoWrite"));
    assert!(plan.contains("StructuredOutput"));
    assert!(!plan.contains("Agent"));

    let verification = allowed_tools_for_subagent("Verification");
    assert!(verification.contains("bash"));
    assert!(verification.contains("PowerShell"));
    assert!(!verification.contains("write_file"));
}

#[derive(Debug)]
struct MockSubagentApiClient {
    calls: usize,
    input_path: String,
}

impl runtime::ApiClient for MockSubagentApiClient {
    fn stream(&mut self, request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
        self.calls += 1;
        match self.calls {
            1 => {
                assert_eq!(request.messages.len(), 1);
                Ok(vec![
                    AssistantEvent::ToolUse {
                        id: "tool-1".to_string(),
                        name: "read_file".to_string(),
                        input: json!({ "path": self.input_path }).to_string(),
                    },
                    AssistantEvent::MessageStop,
                ])
            }
            2 => {
                assert!(request.messages.len() >= 3);
                Ok(vec![
                    AssistantEvent::TextDelta("Scope: completed mock review".to_string()),
                    AssistantEvent::MessageStop,
                ])
            }
            _ => unreachable!("extra mock stream call"),
        }
    }
}

#[test]
fn subagent_runtime_executes_tool_loop_with_isolated_session() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = temp_path("subagent-workspace");
    std::fs::create_dir_all(&root).expect("create workspace");
    let path = root.join("subagent-input.txt");
    std::fs::write(&path, "hello from child").expect("write input file");
    // The delegated read goes through the confined file tools, so the
    // workspace root has to be the directory that holds the file.
    let _cwd = enter_workspace(&root);

    let mut runtime = ConversationRuntime::new(
        Session::new(),
        MockSubagentApiClient { calls: 0, input_path: path.display().to_string() },
        SubagentToolExecutor::new(BTreeSet::from([String::from("read_file")])),
        agent_permission_policy(),
        vec![String::from("system prompt")],
    );

    let summary =
        runtime.run_turn("Inspect the delegated file", None).expect("subagent loop should succeed");

    assert_eq!(final_assistant_text(&summary), "Scope: completed mock review");
    assert!(runtime.session().messages.iter().flat_map(|message| message.blocks.iter()).any(
        |block| matches!(
            block,
            runtime::ContentBlock::ToolResult { output, .. }
                if output.contains("hello from child")
        )
    ));

    let _ = std::fs::remove_file(path);
}

#[test]
fn agent_rejects_blank_required_fields() {
    let missing_description = execute_tool(
        "Agent",
        &json!({
            "description": "  ",
            "prompt": "Inspect"
        }),
    )
    .expect_err("blank description should fail");
    assert!(missing_description.contains("description must not be empty"));

    let missing_prompt = execute_tool(
        "Agent",
        &json!({
            "description": "Inspect branch",
            "prompt": " "
        }),
    )
    .expect_err("blank prompt should fail");
    assert!(missing_prompt.contains("prompt must not be empty"));
}

#[test]
fn notebook_edit_replaces_inserts_and_deletes_cells() {
    let path = temp_path("notebook.ipynb");
    std::fs::write(
            &path,
            r#"{
  "cells": [
    {"cell_type": "code", "id": "cell-a", "metadata": {}, "source": ["print(1)\n"], "outputs": [], "execution_count": null}
  ],
  "metadata": {"kernelspec": {"language": "python"}},
  "nbformat": 4,
  "nbformat_minor": 5
}"#,
        )
        .expect("write notebook");

    let replaced = execute_tool(
        "NotebookEdit",
        &json!({
            "notebook_path": path.display().to_string(),
            "cell_id": "cell-a",
            "new_source": "print(2)\n",
            "edit_mode": "replace"
        }),
    )
    .expect("NotebookEdit replace should succeed");
    let replaced_output: serde_json::Value = serde_json::from_str(&replaced).expect("json");
    assert_eq!(replaced_output["cell_id"], "cell-a");
    assert_eq!(replaced_output["cell_type"], "code");

    let inserted = execute_tool(
        "NotebookEdit",
        &json!({
            "notebook_path": path.display().to_string(),
            "cell_id": "cell-a",
            "new_source": "# heading\n",
            "cell_type": "markdown",
            "edit_mode": "insert"
        }),
    )
    .expect("NotebookEdit insert should succeed");
    let inserted_output: serde_json::Value = serde_json::from_str(&inserted).expect("json");
    assert_eq!(inserted_output["cell_type"], "markdown");
    let appended = execute_tool(
        "NotebookEdit",
        &json!({
            "notebook_path": path.display().to_string(),
            "new_source": "print(3)\n",
            "edit_mode": "insert"
        }),
    )
    .expect("NotebookEdit append should succeed");
    let appended_output: serde_json::Value = serde_json::from_str(&appended).expect("json");
    assert_eq!(appended_output["cell_type"], "code");

    let deleted = execute_tool(
        "NotebookEdit",
        &json!({
            "notebook_path": path.display().to_string(),
            "cell_id": "cell-a",
            "edit_mode": "delete"
        }),
    )
    .expect("NotebookEdit delete should succeed without new_source");
    let deleted_output: serde_json::Value = serde_json::from_str(&deleted).expect("json");
    assert!(deleted_output["cell_type"].is_null());
    assert_eq!(deleted_output["new_source"], "");

    let final_notebook: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read notebook"))
            .expect("valid notebook json");
    let cells = final_notebook["cells"].as_array().expect("cells array");
    assert_eq!(cells.len(), 2);
    assert_eq!(cells[0]["cell_type"], "markdown");
    assert!(cells[0].get("outputs").is_none());
    assert_eq!(cells[1]["cell_type"], "code");
    assert_eq!(cells[1]["source"][0], "print(3)\n");
    let _ = std::fs::remove_file(path);
}

#[test]
fn notebook_edit_rejects_invalid_inputs() {
    let text_path = temp_path("notebook.txt");
    fs::write(&text_path, "not a notebook").expect("write text file");
    let wrong_extension = execute_tool(
        "NotebookEdit",
        &json!({
            "notebook_path": text_path.display().to_string(),
            "new_source": "print(1)\n"
        }),
    )
    .expect_err("non-ipynb file should fail");
    assert!(wrong_extension.contains("Jupyter notebook"));
    let _ = fs::remove_file(&text_path);

    let empty_notebook = temp_path("empty.ipynb");
    fs::write(
            &empty_notebook,
            r#"{"cells":[],"metadata":{"kernelspec":{"language":"python"}},"nbformat":4,"nbformat_minor":5}"#,
        )
        .expect("write empty notebook");

    let missing_source = execute_tool(
        "NotebookEdit",
        &json!({
            "notebook_path": empty_notebook.display().to_string(),
            "edit_mode": "insert"
        }),
    )
    .expect_err("insert without source should fail");
    assert!(missing_source.contains("new_source is required"));

    let missing_cell = execute_tool(
        "NotebookEdit",
        &json!({
            "notebook_path": empty_notebook.display().to_string(),
            "edit_mode": "delete"
        }),
    )
    .expect_err("delete on empty notebook should fail");
    assert!(missing_cell.contains("Notebook has no cells to edit"));
    let _ = fs::remove_file(empty_notebook);
}

#[test]
fn bash_tool_reports_success_exit_failure_timeout_and_background() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let success = execute_tool("bash", &json!({ "command": shell_stdout_command("hello") }))
        .expect("bash should succeed");
    let success_output: serde_json::Value = serde_json::from_str(&success).expect("json");
    assert_eq!(success_output["stdout"].as_str().expect("stdout").trim(), "hello");
    assert_eq!(success_output["interrupted"], false);

    let failure = execute_tool("bash", &json!({ "command": shell_stderr_exit_command("oops", 7) }))
        .expect("bash failure should still return structured output");
    let failure_output: serde_json::Value = serde_json::from_str(&failure).expect("json");
    assert_eq!(failure_output["returnCodeInterpretation"], "exit_code:7");
    assert!(failure_output["stderr"].as_str().expect("stderr").contains("oops"));

    let timeout =
        execute_tool("bash", &json!({ "command": shell_sleep_command(1), "timeout": 10 }))
            .expect("bash timeout should return output");
    let timeout_output: serde_json::Value = serde_json::from_str(&timeout).expect("json");
    assert_eq!(timeout_output["interrupted"], true);
    assert_eq!(timeout_output["returnCodeInterpretation"], "timeout");
    assert!(timeout_output["stderr"]
        .as_str()
        .expect("stderr")
        .contains("Command exceeded timeout"));

    let background = execute_tool(
        "bash",
        &json!({ "command": shell_sleep_command(1), "run_in_background": true }),
    )
    .expect("bash background should succeed");
    let background_output: serde_json::Value = serde_json::from_str(&background).expect("json");
    assert!(background_output["backgroundTaskId"].as_str().is_some());
    assert_eq!(background_output["noOutputExpected"], true);
}

#[test]
fn bash_blocks_shell_text_file_authoring_loops() {
    let output = execute_tool(
            "bash",
            &json!({
                "command": "echo # Review > E:\\code\\PR43-review.md & echo finding >> E:\\code\\PR43-review.md"
            }),
        )
        .expect("bash guard should return structured output");
    let output_json: serde_json::Value = serde_json::from_str(&output).expect("json");
    assert_eq!(
        output_json["returnCodeInterpretation"],
        "preflight_blocked:file_authoring_tool_misuse"
    );
    assert!(output_json["stderr"]
        .as_str()
        .expect("stderr")
        .contains("does not support creating text files through shell commands"));
    assert!(!output_json["stderr"].as_str().expect("stderr").contains("write_file"));
    assert!(!output_json["stderr"].as_str().expect("stderr").contains("edit_file"));
    assert_eq!(output_json["structuredContent"][0]["event"], "lane.blocked");
    assert_eq!(output_json["structuredContent"][0]["failureClass"], "tool_runtime");
    assert_eq!(
        output_json["structuredContent"][0]["data"]["recommendedAction"],
        "external_editor_or_export"
    );
    assert_eq!(
        output_json["returnCodeInterpretation"],
        "preflight_blocked:file_authoring_tool_misuse"
    );
}

#[test]
fn bash_allows_simple_echo_without_file_authoring() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let output = execute_tool("bash", &json!({ "command": shell_stdout_command("hello") }))
        .expect("simple echo should still execute");
    let output_json: serde_json::Value = serde_json::from_str(&output).expect("json");
    assert_eq!(output_json["stdout"].as_str().expect("stdout").trim(), "hello");
    assert_ne!(
        output_json["returnCodeInterpretation"],
        "preflight_blocked:file_authoring_tool_misuse"
    );
}

#[test]
fn bash_blocks_copy_con_interactive_command() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let cases = ["copy con SEGO_WRITE_TEST.md", "copy con file.txt", "COPY CON something"];
    for cmd in cases {
        let output = execute_tool("bash", &json!({ "command": cmd }))
            .expect("bash guard should return structured output");
        let output_json: serde_json::Value = serde_json::from_str(&output).expect("json");
        assert_eq!(
            output_json["returnCodeInterpretation"],
            "preflight_blocked:interactive_command"
        );
        assert!(output_json["stderr"].as_str().expect("stderr").contains("interactive command"));
        assert_eq!(output_json["structuredContent"][0]["data"]["reason"], "interactive_command");
    }
}

#[test]
fn bash_blocks_cmd_wrapped_copy_con() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let output = execute_tool("bash", &json!({ "command": "cmd /c copy con SEGO_WRITE_TEST.md" }))
        .expect("bash guard should return structured output");
    let output_json: serde_json::Value = serde_json::from_str(&output).expect("json");
    assert_eq!(output_json["returnCodeInterpretation"], "preflight_blocked:interactive_command");
}

#[test]
fn bash_blocks_bare_python_powershell_cmd() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let cases = [
        "python",
        "python3",
        "powershell",
        "pwsh",
        "cmd",
        "vim file.txt",
        "nano file.txt",
        "powershell -NoExit",
        "cmd /k echo hello",
        "less file.txt",
        "more file.txt",
    ];
    for cmd in cases {
        let output = execute_tool("bash", &json!({ "command": cmd }))
            .expect("bash guard should return structured output");
        let output_json: serde_json::Value = serde_json::from_str(&output).expect("json");
        assert_eq!(
            output_json["returnCodeInterpretation"], "preflight_blocked:interactive_command",
            "failed for command: {cmd}"
        );
    }
}

#[test]
fn bash_allows_non_interactive_shell_commands() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let cases = [
        ("git status", "SafeReadonly"),
        ("echo hello", "SafeReadonly"),
        ("dir", "SafeReadonly"),
        ("python -c \"print(1)\"", "SafeVerify"),
        ("powershell -Command \"Get-Date\"", "SafeVerify"),
        ("cmd /c \"echo hello\"", "SafeReadonly"),
        ("python -c\"print(1)\"", "SafeVerify"),
        ("python -c'print(1)'", "SafeVerify"),
        ("python -c=print(1)", "SafeVerify"),
        ("powershell -EncodedCommand abc", "SafeVerify"),
        ("echo copy con something", "SafeReadonly"),
    ];
    for (cmd, _expected) in cases {
        // Name the command in the failure. This test once failed on the
        // ubuntu runner with a bare "No such file or directory (os error 2)"
        // and no indication of which of the ten commands produced it, which
        // made the cause unattributable after the fact.
        let output = execute_tool("bash", &json!({ "command": cmd })).unwrap_or_else(|error| {
            panic!("bash should execute `{cmd}`: {error} (PATH={:?})", std::env::var("PATH"))
        });
        let output_json: serde_json::Value = serde_json::from_str(&output).expect("json");
        assert_ne!(
            output_json["returnCodeInterpretation"], "preflight_blocked:interactive_command",
            "non-interactive command '{cmd}' was incorrectly blocked"
        );
    }
}

#[test]
fn bash_workspace_tests_are_blocked_when_branch_is_behind_main() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = temp_path("workspace-test-preflight");
    let original_dir = std::env::current_dir().expect("cwd");
    init_git_repo(&root);
    run_git(&root, &["checkout", "-b", "feature/stale-tests"]);
    run_git(&root, &["checkout", "main"]);
    commit_file(&root, "hotfix.txt", "fix from main\n", "fix: unblock workspace tests");
    run_git(&root, &["checkout", "feature/stale-tests"]);
    std::env::set_current_dir(&root).expect("set cwd");

    let output =
        execute_tool("bash", &json!({ "command": "cargo test --workspace --all-targets" }))
            .expect("preflight should return structured output");
    let output_json: serde_json::Value = serde_json::from_str(&output).expect("json");
    assert_eq!(output_json["returnCodeInterpretation"], "preflight_blocked:branch_divergence");
    assert!(output_json["stderr"]
        .as_str()
        .expect("stderr")
        .contains("branch divergence detected before workspace tests"));
    assert_eq!(output_json["structuredContent"][0]["event"], "branch.stale_against_main");
    assert_eq!(output_json["structuredContent"][0]["failureClass"], "branch_divergence");
    assert_eq!(
        output_json["structuredContent"][0]["data"]["missingCommits"][0],
        "fix: unblock workspace tests"
    );

    std::env::set_current_dir(&original_dir).expect("restore cwd");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn bash_targeted_tests_skip_branch_preflight() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = temp_path("targeted-test-no-preflight");
    let original_dir = std::env::current_dir().expect("cwd");
    init_git_repo(&root);
    run_git(&root, &["checkout", "-b", "feature/targeted-tests"]);
    run_git(&root, &["checkout", "main"]);
    commit_file(&root, "hotfix.txt", "fix from main\n", "fix: only broad tests should block");
    run_git(&root, &["checkout", "feature/targeted-tests"]);
    std::env::set_current_dir(&root).expect("set cwd");

    let output = execute_tool(
        "bash",
        &json!({ "command": "printf 'targeted ok'; cargo test -p runtime stale_branch" }),
    )
    .expect("targeted commands should still execute");
    let output_json: serde_json::Value = serde_json::from_str(&output).expect("json");
    assert_ne!(output_json["returnCodeInterpretation"], "preflight_blocked:branch_divergence");

    std::env::set_current_dir(&original_dir).expect("restore cwd");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn file_tools_cover_read_write_and_edit_behaviors() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = temp_path("fs-suite");
    fs::create_dir_all(&root).expect("create root");
    let original_dir = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(&root).expect("set cwd");

    let write_create = execute_tool(
        "write_file",
        &json!({ "path": "nested/demo.txt", "content": "alpha\nbeta\nalpha\n" }),
    )
    .expect("write create should succeed");
    let write_create_output: serde_json::Value = serde_json::from_str(&write_create).expect("json");
    assert_eq!(write_create_output["type"], "create");
    assert!(root.join("nested/demo.txt").exists());

    let write_update = execute_tool(
        "write_file",
        &json!({ "path": "nested/demo.txt", "content": "alpha\nbeta\ngamma\n" }),
    )
    .expect("write update should succeed");
    let write_update_output: serde_json::Value = serde_json::from_str(&write_update).expect("json");
    assert_eq!(write_update_output["type"], "update");
    assert_eq!(write_update_output["originalFile"], "alpha\nbeta\nalpha\n");

    let read_full = execute_tool("read_file", &json!({ "path": "nested/demo.txt" }))
        .expect("read full should succeed");
    let read_full_output: serde_json::Value = serde_json::from_str(&read_full).expect("json");
    assert_eq!(read_full_output["file"]["content"], "alpha\nbeta\ngamma");
    assert_eq!(read_full_output["file"]["startLine"], 1);

    let read_slice =
        execute_tool("read_file", &json!({ "path": "nested/demo.txt", "offset": 1, "limit": 1 }))
            .expect("read slice should succeed");
    let read_slice_output: serde_json::Value = serde_json::from_str(&read_slice).expect("json");
    assert_eq!(read_slice_output["file"]["content"], "beta");
    assert_eq!(read_slice_output["file"]["startLine"], 2);

    let read_past_end =
        execute_tool("read_file", &json!({ "path": "nested/demo.txt", "offset": 50 }))
            .expect("read past EOF should succeed");
    let read_past_end_output: serde_json::Value =
        serde_json::from_str(&read_past_end).expect("json");
    assert_eq!(read_past_end_output["file"]["content"], "");
    assert_eq!(read_past_end_output["file"]["startLine"], 4);

    let read_error = execute_tool("read_file", &json!({ "path": "missing.txt" }))
        .expect_err("missing file should fail");
    assert!(!read_error.is_empty());

    let edit_once = execute_tool(
        "edit_file",
        &json!({ "path": "nested/demo.txt", "old_string": "alpha", "new_string": "omega" }),
    )
    .expect("single edit should succeed");
    let edit_once_output: serde_json::Value = serde_json::from_str(&edit_once).expect("json");
    assert_eq!(edit_once_output["replaceAll"], false);
    assert_eq!(
        fs::read_to_string(root.join("nested/demo.txt")).expect("read file"),
        "omega\nbeta\ngamma\n"
    );

    execute_tool(
        "write_file",
        &json!({ "path": "nested/demo.txt", "content": "alpha\nbeta\nalpha\n" }),
    )
    .expect("reset file");
    let edit_all = execute_tool(
        "edit_file",
        &json!({
            "path": "nested/demo.txt",
            "old_string": "alpha",
            "new_string": "omega",
            "replace_all": true
        }),
    )
    .expect("replace all should succeed");
    let edit_all_output: serde_json::Value = serde_json::from_str(&edit_all).expect("json");
    assert_eq!(edit_all_output["replaceAll"], true);
    assert_eq!(
        fs::read_to_string(root.join("nested/demo.txt")).expect("read file"),
        "omega\nbeta\nomega\n"
    );

    let edit_same = execute_tool(
        "edit_file",
        &json!({ "path": "nested/demo.txt", "old_string": "omega", "new_string": "omega" }),
    )
    .expect_err("identical old/new should fail");
    assert!(edit_same.contains("must differ"));

    let edit_missing = execute_tool(
        "edit_file",
        &json!({ "path": "nested/demo.txt", "old_string": "missing", "new_string": "omega" }),
    )
    .expect_err("missing substring should fail");
    assert!(edit_missing.contains("old_string not found"));

    std::env::set_current_dir(&original_dir).expect("restore cwd");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn glob_and_grep_tools_cover_success_and_errors() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = temp_path("search-suite");
    fs::create_dir_all(root.join("nested")).expect("create root");
    let original_dir = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(&root).expect("set cwd");

    fs::write(root.join("nested/lib.rs"), "fn main() {}\nlet alpha = 1;\nlet alpha = 2;\n")
        .expect("write rust file");
    fs::write(root.join("nested/notes.txt"), "alpha\nbeta\n").expect("write txt file");

    let globbed = execute_tool("glob_search", &json!({ "pattern": "nested/*.rs" }))
        .expect("glob should succeed");
    let globbed_output: serde_json::Value = serde_json::from_str(&globbed).expect("json");
    assert_eq!(globbed_output["numFiles"], 1);
    assert!(globbed_output["filenames"][0]
        .as_str()
        .expect("filename")
        .replace('\\', "/")
        .ends_with("nested/lib.rs"));

    let glob_error = execute_tool("glob_search", &json!({ "pattern": "[" }))
        .expect_err("invalid glob should fail");
    assert!(!glob_error.is_empty());

    let grep_content = execute_tool(
        "grep_search",
        &json!({
            "pattern": "alpha",
            "path": "nested",
            "glob": "*.rs",
            "output_mode": "content",
            "-n": true,
            "head_limit": 1,
            "offset": 1
        }),
    )
    .expect("grep content should succeed");
    let grep_content_output: serde_json::Value = serde_json::from_str(&grep_content).expect("json");
    assert_eq!(grep_content_output["numFiles"], 0);
    assert!(grep_content_output["appliedLimit"].is_null());
    assert_eq!(grep_content_output["appliedOffset"], 1);
    assert!(grep_content_output["content"].as_str().expect("content").contains("let alpha = 2;"));

    let grep_count = execute_tool(
        "grep_search",
        &json!({ "pattern": "alpha", "path": "nested", "output_mode": "count" }),
    )
    .expect("grep count should succeed");
    let grep_count_output: serde_json::Value = serde_json::from_str(&grep_count).expect("json");
    assert_eq!(grep_count_output["numMatches"], 3);

    let grep_error = execute_tool("grep_search", &json!({ "pattern": "(alpha", "path": "nested" }))
        .expect_err("invalid regex should fail");
    assert!(!grep_error.is_empty());

    std::env::set_current_dir(&original_dir).expect("restore cwd");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn sleep_waits_and_reports_duration() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let started = std::time::Instant::now();
    let result = execute_tool("Sleep", &json!({"duration_ms": 20})).expect("Sleep should succeed");
    let elapsed = started.elapsed();
    let output: serde_json::Value = serde_json::from_str(&result).expect("json");
    assert_eq!(output["duration_ms"], 20);
    assert!(output["message"].as_str().expect("message").contains("Slept for 20ms"));
    assert!(elapsed >= Duration::from_millis(15));
}

#[test]
fn given_excessive_duration_when_sleep_then_rejects_with_error() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = execute_tool("Sleep", &json!({"duration_ms": 999_999_999_u64}));
    let error = result.expect_err("excessive sleep should fail");
    assert!(error.contains("exceeds maximum allowed sleep"));
}

#[test]
fn given_zero_duration_when_sleep_then_succeeds() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let result =
        execute_tool("Sleep", &json!({"duration_ms": 0})).expect("0ms sleep should succeed");
    let output: serde_json::Value = serde_json::from_str(&result).expect("json");
    assert_eq!(output["duration_ms"], 0);
}

#[test]
fn brief_returns_sent_message_and_attachment_metadata() {
    let attachment = std::env::temp_dir().join(format!(
        "clawd-brief-{}.png",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    std::fs::write(&attachment, b"png-data").expect("write attachment");

    let result = execute_tool(
        "SendUserMessage",
        &json!({
            "message": "hello user",
            "attachments": [attachment.display().to_string()],
            "status": "normal"
        }),
    )
    .expect("SendUserMessage should succeed");

    let output: serde_json::Value = serde_json::from_str(&result).expect("json");
    assert_eq!(output["message"], "hello user");
    assert!(output["sentAt"].as_str().is_some());
    assert_eq!(output["attachments"][0]["isImage"], true);
    let _ = std::fs::remove_file(attachment);
}

#[test]
fn config_reads_and_writes_supported_values() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = std::env::temp_dir().join(format!(
        "clawd-config-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    let home = root.join("home");
    let cwd = root.join("cwd");
    std::fs::create_dir_all(home.join(".claw")).expect("home dir");
    std::fs::create_dir_all(cwd.join(".claw")).expect("cwd dir");
    std::fs::write(home.join(".claw").join("settings.json"), r#"{"verbose":false}"#)
        .expect("write global settings");

    let original_home = std::env::var("HOME").ok();
    let original_config_home = std::env::var("CLAW_CONFIG_HOME").ok();
    let original_dir = std::env::current_dir().expect("cwd");
    std::env::set_var("HOME", &home);
    std::env::remove_var("CLAW_CONFIG_HOME");
    std::env::set_current_dir(&cwd).expect("set cwd");

    let get = execute_tool("Config", &json!({"setting": "verbose"})).expect("get config");
    let get_output: serde_json::Value = serde_json::from_str(&get).expect("json");
    assert_eq!(get_output["value"], false);

    let set =
        execute_tool("Config", &json!({"setting": "permissions.defaultMode", "value": "plan"}))
            .expect("set config");
    let set_output: serde_json::Value = serde_json::from_str(&set).expect("json");
    assert_eq!(set_output["operation"], "set");
    assert_eq!(set_output["newValue"], "plan");

    let invalid =
        execute_tool("Config", &json!({"setting": "permissions.defaultMode", "value": "bogus"}))
            .expect_err("invalid config value should error");
    assert!(invalid.contains("Invalid value"));

    let unknown =
        execute_tool("Config", &json!({"setting": "nope"})).expect("unknown setting result");
    let unknown_output: serde_json::Value = serde_json::from_str(&unknown).expect("json");
    assert_eq!(unknown_output["success"], false);

    std::env::set_current_dir(&original_dir).expect("restore cwd");
    match original_home {
        Some(value) => std::env::set_var("HOME", value),
        None => std::env::remove_var("HOME"),
    }
    match original_config_home {
        Some(value) => std::env::set_var("CLAW_CONFIG_HOME", value),
        None => std::env::remove_var("CLAW_CONFIG_HOME"),
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn enter_and_exit_plan_mode_round_trip_existing_local_override() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = std::env::temp_dir().join(format!(
        "clawd-plan-mode-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    let home = root.join("home");
    let cwd = root.join("cwd");
    std::fs::create_dir_all(home.join(".claw")).expect("home dir");
    std::fs::create_dir_all(cwd.join(".claw")).expect("cwd dir");
    std::fs::write(
        cwd.join(".claw").join("settings.local.json"),
        r#"{"permissions":{"defaultMode":"acceptEdits"}}"#,
    )
    .expect("write local settings");

    let original_home = std::env::var("HOME").ok();
    let original_config_home = std::env::var("CLAW_CONFIG_HOME").ok();
    let original_dir = std::env::current_dir().expect("cwd");
    std::env::set_var("HOME", &home);
    std::env::remove_var("CLAW_CONFIG_HOME");
    std::env::set_current_dir(&cwd).expect("set cwd");

    let enter = execute_tool("EnterPlanMode", &json!({})).expect("enter plan mode");
    let enter_output: serde_json::Value = serde_json::from_str(&enter).expect("json");
    assert_eq!(enter_output["changed"], true);
    assert_eq!(enter_output["managed"], true);
    assert_eq!(enter_output["previousLocalMode"], "acceptEdits");
    assert_eq!(enter_output["currentLocalMode"], "plan");

    let local_settings = std::fs::read_to_string(cwd.join(".claw").join("settings.local.json"))
        .expect("local settings after enter");
    assert!(local_settings.contains(r#""defaultMode": "plan""#));
    let state =
        std::fs::read_to_string(cwd.join(".claw").join("tool-state").join("plan-mode.json"))
            .expect("plan mode state");
    assert!(state.contains(r#""hadLocalOverride": true"#));
    assert!(state.contains(r#""previousLocalMode": "acceptEdits""#));

    let exit = execute_tool("ExitPlanMode", &json!({})).expect("exit plan mode");
    let exit_output: serde_json::Value = serde_json::from_str(&exit).expect("json");
    assert_eq!(exit_output["changed"], true);
    assert_eq!(exit_output["managed"], false);
    assert_eq!(exit_output["previousLocalMode"], "acceptEdits");
    assert_eq!(exit_output["currentLocalMode"], "acceptEdits");

    let local_settings = std::fs::read_to_string(cwd.join(".claw").join("settings.local.json"))
        .expect("local settings after exit");
    assert!(local_settings.contains(r#""defaultMode": "acceptEdits""#));
    assert!(!cwd.join(".claw").join("tool-state").join("plan-mode.json").exists());

    std::env::set_current_dir(&original_dir).expect("restore cwd");
    match original_home {
        Some(value) => std::env::set_var("HOME", value),
        None => std::env::remove_var("HOME"),
    }
    match original_config_home {
        Some(value) => std::env::set_var("CLAW_CONFIG_HOME", value),
        None => std::env::remove_var("CLAW_CONFIG_HOME"),
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn exit_plan_mode_clears_override_when_enter_created_it_from_empty_local_state() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = std::env::temp_dir().join(format!(
        "clawd-plan-mode-empty-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    let home = root.join("home");
    let cwd = root.join("cwd");
    std::fs::create_dir_all(home.join(".claw")).expect("home dir");
    std::fs::create_dir_all(cwd.join(".claw")).expect("cwd dir");

    let original_home = std::env::var("HOME").ok();
    let original_config_home = std::env::var("CLAW_CONFIG_HOME").ok();
    let original_dir = std::env::current_dir().expect("cwd");
    std::env::set_var("HOME", &home);
    std::env::remove_var("CLAW_CONFIG_HOME");
    std::env::set_current_dir(&cwd).expect("set cwd");

    let enter = execute_tool("EnterPlanMode", &json!({})).expect("enter plan mode");
    let enter_output: serde_json::Value = serde_json::from_str(&enter).expect("json");
    assert_eq!(enter_output["previousLocalMode"], serde_json::Value::Null);
    assert_eq!(enter_output["currentLocalMode"], "plan");

    let exit = execute_tool("ExitPlanMode", &json!({})).expect("exit plan mode");
    let exit_output: serde_json::Value = serde_json::from_str(&exit).expect("json");
    assert_eq!(exit_output["changed"], true);
    assert_eq!(exit_output["currentLocalMode"], serde_json::Value::Null);

    let local_settings = std::fs::read_to_string(cwd.join(".claw").join("settings.local.json"))
        .expect("local settings after exit");
    let local_settings_json: serde_json::Value =
        serde_json::from_str(&local_settings).expect("valid settings json");
    assert_eq!(
        local_settings_json.get("permissions"),
        None,
        "permissions override should be removed on exit"
    );
    assert!(!cwd.join(".claw").join("tool-state").join("plan-mode.json").exists());

    std::env::set_current_dir(&original_dir).expect("restore cwd");
    match original_home {
        Some(value) => std::env::set_var("HOME", value),
        None => std::env::remove_var("HOME"),
    }
    match original_config_home {
        Some(value) => std::env::set_var("CLAW_CONFIG_HOME", value),
        None => std::env::remove_var("CLAW_CONFIG_HOME"),
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn structured_output_echoes_input_payload() {
    let result = execute_tool("StructuredOutput", &json!({"ok": true, "items": [1, 2, 3]}))
        .expect("StructuredOutput should succeed");
    let output: serde_json::Value = serde_json::from_str(&result).expect("json");
    assert_eq!(output["data"], "Structured output provided successfully");
    assert_eq!(output["structured_output"]["ok"], true);
    assert_eq!(output["structured_output"]["items"][1], 2);
}

#[test]
fn given_empty_payload_when_structured_output_then_rejects_with_error() {
    let result = execute_tool("StructuredOutput", &json!({}));
    let error = result.expect_err("empty payload should fail");
    assert!(error.contains("must not be empty"));
}

#[test]
fn repl_executes_python_code() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // A host without a usable Python cannot construct this test's fixture,
    // and the failure would look like a broken REPL rather than a missing
    // interpreter. This was observed on a Windows runner where resolution
    // found only the Store alias stub. Say so instead of reporting a
    // product defect.
    if !python_is_usable() {
        eprintln!("skipping repl_executes_python_code: no usable Python interpreter on this host");
        return;
    }
    let result = execute_tool(
        "REPL",
        &json!({"language": "python", "code": "print(1 + 1)", "timeout_ms": 500}),
    )
    .expect("REPL should succeed");
    let output: serde_json::Value = serde_json::from_str(&result).expect("json");
    assert_eq!(output["language"], "python");
    assert_eq!(output["exitCode"], 0);
    assert!(output["stdout"].as_str().expect("stdout").contains('2'));
}

/// True when some Python candidate actually runs, not merely resolves.
fn python_is_usable() -> bool {
    let candidates: &[&str] =
        if cfg!(windows) { &["python", "py", "python3"] } else { &["python3", "python"] };
    candidates.iter().any(|candidate| {
        std::process::Command::new(candidate)
            .args(["-c", "print(1)"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

#[test]
fn given_empty_code_when_repl_then_rejects_with_error() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = execute_tool("REPL", &json!({"language": "python", "code": "   "}));

    let error = result.expect_err("empty REPL code should fail");
    assert!(error.contains("code must not be empty"));
}

#[test]
fn given_unsupported_language_when_repl_then_rejects_with_error() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let result = execute_tool("REPL", &json!({"language": "ruby", "code": "puts 1"}));

    let error = result.expect_err("unsupported REPL language should fail");
    assert!(error.contains("unsupported REPL language: ruby"));
}

#[test]
fn given_timeout_ms_when_repl_blocks_then_returns_timeout_error() {
    let result = execute_tool(
        "REPL",
        &json!({
            "language": "python",
            "code": "import time\ntime.sleep(1)",
            "timeout_ms": 10
        }),
    );

    let error = result.expect_err("timed out REPL execution should fail");
    assert!(error.contains("REPL execution exceeded timeout of 10 ms"));
}

#[test]
fn powershell_runs_via_stub_shell() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join(format!(
        "clawd-pwsh-bin-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create dir");
    let script = if cfg!(windows) { dir.join("pwsh.cmd") } else { dir.join("pwsh") };
    #[cfg(windows)]
        let script_body = "@echo off\r\n:loop\r\nif \"%1\"==\"-Command\" goto found\r\nif \"%1\"==\"\" goto done\r\nshift\r\ngoto loop\r\n:found\r\nshift\r\necho|set /p=\"pwsh:%1\"\r\n:done\r\n";
    #[cfg(unix)]
    let script_body = r#"#!/bin/sh
while [ "$1" != "-Command" ] && [ $# -gt 0 ]; do shift; done
shift
printf 'pwsh:%s' "$1"
"#;
    std::fs::write(&script, script_body).expect("write script");
    #[cfg(unix)]
    std::process::Command::new("/bin/chmod").arg("+x").arg(&script).status().expect("chmod");
    let original_path = std::env::var("PATH").unwrap_or_default();
    let separator = if cfg!(windows) { ";" } else { ":" };
    std::env::set_var("PATH", format!("{}{separator}{original_path}", dir.display()));

    let result =
        execute_tool("PowerShell", &json!({"command": "Write-Output hello", "timeout": 1000}))
            .expect("PowerShell should succeed");

    let background = execute_tool(
        "PowerShell",
        &json!({"command": "Write-Output hello", "run_in_background": true}),
    )
    .expect("PowerShell background should succeed");

    std::env::set_var("PATH", original_path);
    let _ = std::fs::remove_dir_all(dir);

    let output: serde_json::Value = serde_json::from_str(&result).expect("json");
    assert_eq!(
        output["stdout"].as_str().expect("stdout").replace('"', ""),
        "pwsh:Write-Output hello"
    );
    assert!(output["stderr"].as_str().expect("stderr").is_empty());

    let background_output: serde_json::Value = serde_json::from_str(&background).expect("json");
    assert!(background_output["backgroundTaskId"].as_str().is_some());
    assert_eq!(background_output["backgroundedByUser"], true);
    assert_eq!(background_output["assistantAutoBackgrounded"], false);
}

#[test]
fn powershell_errors_when_shell_is_missing() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let original_path = std::env::var("PATH").unwrap_or_default();
    let empty_dir = std::env::temp_dir().join(format!(
        "clawd-empty-bin-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&empty_dir).expect("create empty dir");
    std::env::set_var("PATH", empty_dir.display().to_string());

    let err = execute_tool("PowerShell", &json!({"command": "Write-Output hello"}))
        .expect_err("PowerShell should fail when shell is missing");

    std::env::set_var("PATH", original_path);
    let _ = std::fs::remove_dir_all(empty_dir);

    assert!(err.contains("PowerShell executable not found"));
}

fn read_only_registry() -> super::GlobalToolRegistry {
    use runtime::permission_enforcer::PermissionEnforcer;
    use runtime::PermissionPolicy;

    let policy = mvp_tool_specs()
        .into_iter()
        .fold(PermissionPolicy::new(runtime::PermissionMode::ReadOnly), |policy, spec| {
            policy.with_tool_requirement(spec.name, spec.required_permission)
        });
    let mut registry = super::GlobalToolRegistry::builtin();
    registry.set_enforcer(PermissionEnforcer::new(policy));
    registry
}

#[test]
fn given_read_only_enforcer_when_bash_then_denied() {
    let registry = read_only_registry();
    let err = registry
        .execute("bash", &json!({ "command": "echo hi" }))
        .expect_err("bash should be denied in read-only mode");
    assert!(err.contains("current mode is read-only"), "should cite active mode: {err}");
}

#[test]
fn given_read_only_enforcer_when_write_file_then_denied() {
    let registry = read_only_registry();
    let err = registry
        .execute("write_file", &json!({ "path": "/tmp/x.txt", "content": "x" }))
        .expect_err("write_file should be denied in read-only mode");
    assert!(err.contains("current mode is read-only"), "should cite active mode: {err}");
}

#[test]
fn given_read_only_enforcer_when_edit_file_then_denied() {
    let registry = read_only_registry();
    let err = registry
        .execute(
            "edit_file",
            &json!({ "path": "/tmp/x.txt", "old_string": "a", "new_string": "b" }),
        )
        .expect_err("edit_file should be denied in read-only mode");
    assert!(err.contains("current mode is read-only"), "should cite active mode: {err}");
}

#[test]
fn given_read_only_enforcer_when_read_file_then_not_permission_denied() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = temp_path("perm-read");
    fs::create_dir_all(&root).expect("create root");
    let file = root.join("readable.txt");
    fs::write(&file, "content\n").expect("write test file");

    // read_file is confined to the workspace root, which is the working
    // directory, so the read has to happen from inside the workspace.
    let cwd = enter_workspace(&root);

    let registry = read_only_registry();
    let result = registry.execute("read_file", &json!({ "path": "readable.txt" }));
    assert!(result.is_ok(), "read_file should be allowed: {result:?}");

    drop(cwd);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn given_read_only_enforcer_when_glob_search_then_not_permission_denied() {
    let registry = read_only_registry();
    let result = registry.execute("glob_search", &json!({ "pattern": "*.rs" }));
    assert!(result.is_ok(), "glob_search should be allowed in read-only mode: {result:?}");
}

#[test]
fn given_no_enforcer_when_bash_then_executes_normally() {
    let _guard = env_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let registry = super::GlobalToolRegistry::builtin();
    let result = registry
        .execute("bash", &json!({ "command": shell_stdout_command("ok") }))
        .expect("bash should succeed without enforcer");
    let output: serde_json::Value = serde_json::from_str(&result).expect("json");
    assert_eq!(output["stdout"].as_str().expect("stdout").trim(), "ok");
}

#[test]
fn run_task_packet_creates_packet_backed_task() {
    let result = run_task_packet(TaskPacket {
        objective: "Ship packetized runtime task".to_string(),
        scope: "runtime/task system".to_string(),
        repo: "claw-code-parity".to_string(),
        branch_policy: "origin/main only".to_string(),
        acceptance_tests: vec![
            "cargo build --workspace".to_string(),
            "cargo test --workspace".to_string(),
        ],
        commit_policy: "single commit".to_string(),
        reporting_contract: "print build/test result and sha".to_string(),
        escalation_policy: "manual escalation".to_string(),
    })
    .expect("task packet should create a task");

    let output: serde_json::Value = serde_json::from_str(&result).expect("json");
    assert_eq!(output["status"], "created");
    assert_eq!(output["prompt"], "Ship packetized runtime task");
    assert_eq!(output["description"], "runtime/task system");
    assert_eq!(output["task_packet"]["repo"], "claw-code-parity");
    assert_eq!(output["task_packet"]["acceptance_tests"][1], "cargo test --workspace");
}

struct TestServer {
    addr: SocketAddr,
    shutdown: Option<std::sync::mpsc::Sender<()>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl TestServer {
    fn spawn(handler: Arc<dyn Fn(&str) -> HttpResponse + Send + Sync + 'static>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        listener.set_nonblocking(true).expect("set nonblocking listener");
        let addr = listener.local_addr().expect("local addr");
        let (tx, rx) = std::sync::mpsc::channel::<()>();

        let handle = thread::spawn(move || loop {
            if rx.try_recv().is_ok() {
                break;
            }

            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut buffer = [0_u8; 4096];
                    let size = stream.read(&mut buffer).expect("read request");
                    let request = String::from_utf8_lossy(&buffer[..size]).into_owned();
                    let request_line = request.lines().next().unwrap_or_default().to_string();
                    let response = handler(&request_line);
                    stream.write_all(response.to_bytes().as_slice()).expect("write response");
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("server accept failed: {error}"),
            }
        });

        Self { addr, shutdown: Some(tx), handle: Some(handle) }
    }

    fn addr(&self) -> SocketAddr {
        self.addr
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            handle.join().expect("join test server");
        }
    }
}

struct HttpResponse {
    status: u16,
    reason: &'static str,
    content_type: &'static str,
    body: String,
    location: Option<String>,
}

impl HttpResponse {
    fn html(status: u16, reason: &'static str, body: &str) -> Self {
        Self {
            status,
            reason,
            content_type: "text/html; charset=utf-8",
            body: body.to_string(),
            location: None,
        }
    }

    fn text(status: u16, reason: &'static str, body: &str) -> Self {
        Self {
            status,
            reason,
            content_type: "text/plain; charset=utf-8",
            body: body.to_string(),
            location: None,
        }
    }

    /// A `302` carrying a `Location`, so redirection handling can be
    /// exercised against a real server rather than reasoned about.
    fn redirect(location: &str) -> Self {
        Self {
            status: 302,
            reason: "Found",
            content_type: "text/plain; charset=utf-8",
            body: String::new(),
            location: Some(location.to_string()),
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        let location = self
            .location
            .as_ref()
            .map(|value| format!("Location: {value}\r\n"))
            .unwrap_or_default();
        format!(
                "HTTP/1.1 {} {}\r\nContent-Type: {}\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                self.status,
                self.reason,
                self.content_type,
                location,
                self.body.len(),
                self.body
            )
            .into_bytes()
    }
}
