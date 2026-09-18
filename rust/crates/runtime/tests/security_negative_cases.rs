//! DEV-SEC-13: every fixed security item must carry a dynamic negative case,
//! and CI must run it as a blocking gate.
//!
//! The security fixes in this branch each shipped with a test that drives the
//! real code with the input the item was about - a redirect to a metadata
//! address, a tampered download, a hook that tries to escalate, a path that
//! escapes the workspace. Those cases are what make the fixes verifiable rather
//! than asserted, and they are easy to lose: a refactor renames one, a module
//! moves, and nothing notices until the vulnerability is back.
//!
//! This suite is the net for that. It does not re-test the behaviour; it fails
//! if the *evidence for an item disappears*, and it fails if CI stops running
//! the suite as a blocking gate. An item with no case yet is listed separately
//! and explicitly, so "not covered" is visible instead of implied.
//!
//! ## What this net does not check, and got wrong once
//!
//! It matches test **names**, which means the evidence it insists on can be a
//! test of code that never runs. `DEV-SEC-08` was in exactly that state: its
//! required case `workspace_write_blocks_system_paths` lived in
//! `bash_validation`, a module with no caller, so the item was recorded as fixed
//! on the strength of a *dormant* validator's verdict - and this net, which was
//! built to catch evidence going missing, happily held that name for as long as
//! the module existed. Retiring the module is what exposed it, not this check.
//!
//! So: a name here proves that someone wrote a case, not that the case covers the
//! path that runs. When an item's evidence lives near a boundary like that, the
//! requirement should name a test that drives the live code - which is what
//! `DEV-SEC-08` now does (see its entry below).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    // rust/crates/runtime -> rust/ -> repository root
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// Every tracked Rust source under `rust/`, excluding build output.
fn rust_sources() -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![repo_root().join("rust")];
    while let Some(directory) = pending.pop() {
        if directory.ends_with("target") {
            continue;
        }
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                found.push(path);
            }
        }
    }
    found
}

/// Names of functions that carry a `#[test]` / `#[tokio::test]` attribute.
fn test_function_names() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for path in rust_sources() {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            let trimmed = line.trim_start();
            let Some(rest) = trimmed.strip_prefix("fn ") else {
                continue;
            };
            let Some(name) = rest.split(['(', '<']).next() else {
                continue;
            };
            // The attribute sits on one of the two lines above; allow for a
            // further attribute such as `#[should_panic]`.
            let attributed = lines[index.saturating_sub(2)..index].iter().any(|candidate| {
                candidate.trim_start().starts_with("#[test")
                    || candidate.trim_start().starts_with("#[tokio::test")
            });
            if attributed {
                names.insert(name.trim().to_string());
            }
        }
    }
    names
}

/// The dynamic negative case(s) that back each fixed security item.
///
/// Item ids are `SEG-DEV-001`; each requirement is a test that fails, or that
/// asserts a refusal, on the exact input the item was raised about.
const REQUIRED_CASES: &[(&str, &[&str])] = &[
    (
        "DEV-SEC-01 tampered install/update is refused, not installed",
        &[
            "update_verification_refuses_and_discards_a_tampered_download",
            "update_verification_refuses_a_checksums_file_without_the_asset",
            "update_verification_accepts_a_matching_download",
            "update_verification_skips_only_with_the_explicit_override",
        ],
    ),
    (
        "DEV-SEC-02 a path escaping the workspace is refused",
        &["live_file_entries_refuse_paths_outside_the_workspace"],
    ),
    (
        "DEV-SEC-03 internal targets and redirected hops are refused",
        &[
            "web_fetch_refuses_loopback_private_and_metadata_targets",
            "web_fetch_revalidates_every_hop_instead_of_following_blindly",
            "web_fetch_follows_a_permitted_redirect_and_counts_it",
            "web_fetch_stops_after_the_redirect_cap",
        ],
    ),
    (
        "DEV-SEC-04 an MCP child does not inherit credentials",
        &["spawned_child_sees_only_allowlisted_variables_plus_its_own_config"],
    ),
    (
        "DEV-SEC-05 a hook cannot read credentials or escalate its own authority",
        &[
            "hook_process_does_not_inherit_the_parent_environment",
            "hook_allow_override_cannot_escalate_beyond_the_active_mode",
            "hook_allow_override_cannot_override_an_explicit_deny_rule",
        ],
    ),
    (
        "DEV-SEC-06 an unregistered tool is denied in every mode",
        &["unregistered_tools_are_denied_in_every_mode"],
    ),
    (
        "DEV-SEC-07 executors, writers and flag=value forms are not read-only",
        &[
            "read_only_heuristic_excludes_code_execution_and_writes",
            "read_only_heuristic_gates_multipurpose_tools_by_subcommand",
            "read_only_heuristic_rejects_flag_value_forms_that_change_remote_state",
            "read_only_heuristic_rejects_redirection_and_in_place_writes",
        ],
    ),
    (
        "DEV-SEC-08 a system-path write is blocked, not merely warned",
        // Was `workspace_write_blocks_system_paths`, which lived in
        // `bash_validation` - a module with no caller at all. That test asserted
        // the verdict of a *dormant* validator, so it proved the intent without
        // proving that anything enforced it; the item was recorded as fixed on
        // the strength of code that never ran. Retiring that module exposed it.
        //
        // `workspace_write_denies_outside_workspace` is the stronger evidence and
        // the right one: it drives the live enforcer and asserts
        // `EnforcementResult::Denied` for `check_file_write("/etc/passwd", ...)`
        // in workspace-write mode - a deny on the path that runs, not a warn.
        &["workspace_write_denies_outside_workspace"],
    ),
    (
        "DEV-SEC-09 a chained or out-of-tree command is not Sego metadata",
        &["metadata_write_does_not_cover_chained_or_outside_writes"],
    ),
    (
        // Two halves: the credential store is owner-only (Unix-only case), and
        // a session transcript is redacted before it is written (cross-platform,
        // so Windows exercises this item through the redaction cases).
        "DEV-SEC-10 stored credentials are owner-only and transcripts are redacted",
        &[
            "saved_credentials_are_not_readable_by_other_users",
            "redacts_credentials_that_have_a_recognisable_shape",
            "redacts_secrets_assigned_to_a_credential_named_key",
            "redacts_a_pem_private_key_block_including_its_body",
            "leaves_ordinary_code_and_prose_alone",
            "persisted_transcripts_redact_messages_but_keep_the_record_readable",
            "appended_messages_are_redacted_too",
            "redaction_is_idempotent",
        ],
    ),
    (
        "DEV-SEC-11 the isolation a platform lacks is declared, not implied",
        &["platform_isolation_is_declared_even_when_nothing_is_requested"],
    ),
    ("DEV-SEC-15 fetched content is marked untrusted", &["web_fetch_returns_prompt_aware_summary"]),
    (
        "DEV-SEC-14 a non-crates.io or git dependency is detected",
        &[
            "every_resolved_dependency_comes_from_crates_io",
            "a_git_or_alternate_registry_dependency_is_detected",
            "no_manifest_declares_a_git_dependency",
            "the_dependency_audit_policy_is_present_and_not_hollowed_out",
        ],
    ),
    (
        // The mechanism half: tree-kill tolerates an already-exited target,
        // pid 0 is never signalled, a live tree reports Reclaimed, a refused
        // signal reports a receipt instead of being swallowed, and a killed
        // group leaves no surviving grandchild - on Unix and on Windows, which
        // is where the implementation differs. The ledger half (DEV-CON-08) is
        // covered by the recording case: it records while a task is active and
        // stays silent when none is.
        "DEV-SEC-16 a timeout or cancel reclaims the spawned process tree",
        &[
            "killing_an_already_exited_pid_is_not_an_error",
            "pid_zero_is_never_signalled",
            "killing_a_live_tree_is_reported_as_reclaimed",
            "a_refused_kill_produces_a_receipt_naming_the_pid",
            "only_a_failure_produces_a_receipt",
            "a_timeout_reclaims_the_grandchild_by_its_own_pid",
            "a_timeout_reclaims_the_grandchild_on_windows_too",
            "the_ledger_records_a_spawn_only_while_a_task_is_active",
        ],
    ),
];

/// Items whose negative case cannot exist yet, with the reason. Kept separate so
/// an absent case is recorded rather than quietly passing.
const PENDING_CASES: &[(&str, &str)] = &[];

#[test]
fn every_fixed_security_item_still_carries_its_negative_case() {
    let names = test_function_names();
    assert!(
        !names.is_empty(),
        "no test functions were found; the source scan is broken and this check would be vacuous"
    );

    let mut missing = Vec::new();
    for (item, required) in REQUIRED_CASES {
        for name in *required {
            if !names.contains(*name) {
                missing.push(format!("{item}: {name}"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "a security item lost the test that proves it was fixed:\n  {}",
        missing.join("\n  ")
    );
}

#[test]
fn pending_security_items_are_recorded_rather_than_assumed() {
    // A guard on the guard. DEV-SEC-16 was the last item without a case, so the
    // pending list is now expected to be empty. The assertion points the other
    // way from before on purpose: a non-empty list is no longer normal, and an
    // entry added back must carry a reason, so "deferred" can never quietly
    // become "forgotten".
    assert!(
        PENDING_CASES.is_empty(),
        "every security item now has its negative case; re-adding a pending entry requires clearing it again:\n  {}",
        PENDING_CASES
            .iter()
            .map(|(item, _)| (*item).to_string())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
    for (item, reason) in PENDING_CASES {
        assert!(!reason.is_empty(), "{item} must record why its case is missing");
    }
}

/// The lines of one top-level job in a workflow, from its key to the next key.
fn job_block(workflow: &str, job: &str) -> String {
    let lines: Vec<&str> = workflow.lines().collect();
    let key = format!("{job}:");
    let Some(start) = lines.iter().position(|line| line.trim_end() == format!("  {key}")) else {
        return String::new();
    };
    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, line)| {
            line.starts_with("  ") && !line.starts_with("   ") && line.trim_end().ends_with(':')
        })
        .map_or(lines.len(), |(index, _)| index);
    lines[start..end].join("\n")
}

#[test]
fn ci_runs_the_suite_without_an_escape_hatch() {
    let workflow_path = repo_root().join(".github/workflows/rust-ci.yml");
    let text = fs::read_to_string(&workflow_path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", workflow_path.display()));

    let test_step = text
        .lines()
        .position(|line| line.contains("cargo test --workspace"))
        .expect("CI must run the workspace test suite; the negative cases ride on it");

    // The step must not be advisory, and the job must not be allowed to fail.
    let around = text.lines().skip(test_step).take(4).collect::<Vec<_>>().join("\n");
    assert!(!around.contains("continue-on-error"), "the test step must stay blocking:\n{around}");
    assert!(
        !text.contains("cargo test --workspace || true"),
        "the suite must not be made advisory with `|| true`"
    );

    // Scoped to the jobs that run tests, and checked on the job block rather
    // than on single lines: an earlier version of this assertion compared a line
    // against two conditions that cannot both hold, so it could never fail.
    for job in ["test-workspace", "test-platforms"] {
        let block = job_block(&text, job);
        assert!(!block.is_empty(), "{job} must still exist in the workflow");
        assert!(
            !block.contains("continue-on-error"),
            "{job} runs the tests that prove the security fixes; it must not be advisory:\n{block}"
        );
    }

    // What this test cannot assert, and must not be read as asserting: whether a
    // red run stops a merge. That is a repository setting, not a workflow one --
    // `main` carries no branch protection today, so a failing run reports a
    // result rather than blocking anything. See DEV-GOV-05.
}

/// The lines of the workflow's `on:` block, which is where a trigger is narrowed.
fn trigger_block(workflow: &str) -> String {
    let lines: Vec<&str> = workflow.lines().collect();
    let Some(start) = lines.iter().position(|line| line.trim_end() == "on:") else {
        return String::new();
    };
    let end = lines
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, line)| !line.starts_with(' ') && !line.is_empty() && !line.starts_with('#'))
        .map_or(lines.len(), |(index, _)| index);
    lines[start..end].join("\n")
}

#[test]
fn no_change_can_fall_outside_the_crates_that_hold_the_cases() {
    // This used to assert that the `paths` filter still listed `rust/**`, because
    // a change to a crate holding a negative case that stopped triggering CI
    // would leave the case existing but never running on a pull request.
    //
    // The filter is gone, so that concern is now covered more strongly: with no
    // filter there is no change that can fall outside. The guard is kept rather
    // than deleted, because re-adding one would reintroduce two problems at once
    // - changes that run nothing, and, while a check is required for merging, a
    // pull request waiting forever on a check that never reports.
    let workflow = repo_root().join(".github/workflows/rust-ci.yml");
    let text = fs::read_to_string(&workflow).expect("read workflow");
    let triggers = trigger_block(&text);
    assert!(!triggers.is_empty(), "the workflow must still declare triggers:\n{text}");
    for narrowing in ["paths:", "paths-ignore:"] {
        assert!(
            !triggers.contains(narrowing),
            "a `{narrowing}` entry would let a change run nothing, and would leave a required \
             check permanently unreported; if one is genuinely wanted, this guard and the branch \
             protection settings have to be reconsidered together:\n{triggers}"
        );
    }
}
