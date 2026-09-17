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
        &["workspace_write_blocks_system_paths"],
    ),
    (
        "DEV-SEC-09 a chained or out-of-tree command is not Sego metadata",
        &["metadata_write_does_not_cover_chained_or_outside_writes"],
    ),
    (
        "DEV-SEC-10 stored credentials are owner-only",
        &["saved_credentials_are_not_readable_by_other_users"],
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
];

/// Items whose negative case cannot exist yet, with the reason. Kept separate so
/// an absent case is recorded rather than quietly passing.
const PENDING_CASES: &[(&str, &str)] = &[(
    "DEV-SEC-16",
    "process-tree reclamation is in flight in another window (the case belongs with that change)",
)];

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
    // A guard on the guard: if someone removes the pending list while the items
    // are still open, the omission should be deliberate.
    assert!(
        !PENDING_CASES.is_empty(),
        "either the remaining security items gained their cases, or this list was emptied by mistake"
    );
    for (item, reason) in PENDING_CASES {
        assert!(!reason.is_empty(), "{item} must record why its case is missing");
    }
}

/// The lines of one top-level job in a workflow, from its key to the next key.
fn job_block<'a>(workflow: &'a str, job: &str) -> String {
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

#[test]
fn the_path_filter_still_covers_the_crates_that_hold_the_cases() {
    // If a change to a crate containing a negative case stopped triggering CI,
    // the case would exist but never run on a pull request.
    let workflow = repo_root().join(".github/workflows/rust-ci.yml");
    let text = fs::read_to_string(&workflow).expect("read workflow");
    assert!(text.contains("- rust/**"), "the path filter must keep covering the Rust workspace");
    assert!(
        text.contains(".github/workflows/**"),
        "a change to the workflow itself must trigger the workflow"
    );
}
