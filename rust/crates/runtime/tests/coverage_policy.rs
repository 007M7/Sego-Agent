//! Coverage policy (DEV-QA-04), enforced offline.
//!
//! The repository had no coverage measurement, so "the tests are good" was an
//! assertion rather than a number. Measuring needs `cargo-llvm-cov` and a full
//! instrumented build, which is why the measurement itself runs in the
//! `coverage` CI job.
//!
//! This file covers what can be decided from the repository alone, so it is
//! blocking on every `cargo test --workspace`:
//!
//! 1. the recorded baseline is present, dated, and attributable to a command
//!    and a tool version, so a number cannot be invented later;
//! 2. every workspace crate has a recorded measurement and a floor, so a new
//!    crate cannot join the workspace unmeasured or exempt itself by omission;
//! 3. no floor sits above the measurement it is derived from - a floor that is
//!    already unreachable does not measure anything, it just fails;
//! 4. the CI job runs at the recorded floor, so the number in the file and the
//!    number in CI cannot drift apart.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

fn rust_dir() -> PathBuf {
    // rust/crates/runtime -> rust/
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn repo_root() -> PathBuf {
    rust_dir().join("..")
}

fn baseline() -> Value {
    let path = rust_dir().join("coverage-baseline.json");
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|error| panic!("{} is not valid JSON: {error}", path.display()))
}

/// Crate directory names declared by the workspace manifest.
fn workspace_crates() -> BTreeSet<String> {
    let path = rust_dir().join("Cargo.toml");
    let manifest = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));

    let mut crates = BTreeSet::new();
    for line in manifest.lines() {
        let trimmed = line.trim();
        // `members = ["crates/*"]` is the only shape in use. A literal member
        // path is handled too so a future non-glob member is still seen.
        if !trimmed.starts_with("members") {
            continue;
        }
        let Some(list) = trimmed.split('[').nth(1).and_then(|rest| rest.split(']').next()) else {
            continue;
        };
        for entry in list.split(',') {
            let entry = entry.trim().trim_matches('"').trim();
            if entry.is_empty() {
                continue;
            }
            if let Some(prefix) = entry.strip_suffix("/*") {
                let directory = rust_dir().join(prefix);
                let entries = fs::read_dir(&directory)
                    .unwrap_or_else(|error| panic!("cannot read {}: {error}", directory.display()));
                for item in entries.filter_map(Result::ok) {
                    if item.path().join("Cargo.toml").exists() {
                        if let Some(name) = item.file_name().to_str() {
                            crates.insert(name.to_string());
                        }
                    }
                }
            } else if let Some(name) = Path::new(entry).file_name().and_then(|value| value.to_str())
            {
                crates.insert(name.to_string());
            }
        }
    }
    assert!(
        !crates.is_empty(),
        "no workspace crates were discovered; the members parsing must be broken"
    );
    crates
}

fn measured_per_crate(document: &Value) -> &serde_json::Map<String, Value> {
    document
        .get("measured_per_crate_lines_percent")
        .and_then(Value::as_object)
        .expect("the baseline must record a measurement per crate")
}

fn floor_per_crate(document: &Value) -> &serde_json::Map<String, Value> {
    document
        .get("floor")
        .and_then(|floor| floor.get("per_crate"))
        .and_then(Value::as_object)
        .expect("the baseline must record a floor per crate")
}

#[test]
fn the_baseline_records_how_it_was_obtained() {
    let document = baseline();
    let measured = document
        .get("measured")
        .and_then(Value::as_object)
        .expect("the baseline must record a `measured` section");

    for key in ["date", "tool", "command", "platform", "measured_on", "caveat"] {
        let value = measured.get(key).and_then(Value::as_str).unwrap_or_default();
        assert!(
            !value.trim().is_empty(),
            "`measured.{key}` must be filled in: a number without its provenance cannot be checked later"
        );
    }

    let lines = measured
        .get("lines_total")
        .and_then(Value::as_u64)
        .expect("the baseline must record the total line count");
    assert!(
        lines > 10_000,
        "a total of {lines} lines is too small for this workspace; the baseline looks misplaced"
    );
}

#[test]
fn every_workspace_crate_is_measured_and_floored() {
    let document = baseline();
    let measured = measured_per_crate(&document);
    let floors = floor_per_crate(&document);

    for name in workspace_crates() {
        assert!(
            measured.contains_key(&name),
            "{name} is a workspace crate with no recorded coverage. Measure it, or record why it is not judged."
        );
        assert!(
            floors.contains_key(&name),
            "{name} is a workspace crate with no coverage floor, so its coverage can fall to zero unnoticed"
        );
    }

    for name in floors.keys() {
        assert!(
            measured.contains_key(name),
            "`{name}` has a floor but no measurement; the floor cannot be justified"
        );
    }
}

#[test]
fn no_floor_sits_above_the_measurement_it_comes_from() {
    let document = baseline();
    let measured = measured_per_crate(&document);
    let floors = floor_per_crate(&document);

    let mut violations = Vec::new();
    for (name, floor) in floors {
        let floor = floor.as_f64().unwrap_or_else(|| panic!("floor for `{name}` must be a number"));
        let measured = measured.get(name).and_then(Value::as_f64).unwrap_or_else(|| {
            panic!("no measurement recorded for `{name}`, so its floor cannot be checked")
        });
        if floor > measured {
            violations.push(format!("{name}: floor {floor} > measured {measured}"));
        }
    }
    assert!(
        violations.is_empty(),
        "a floor above the measured value fails before any change is made:\n{}",
        violations.join("\n")
    );

    let overall = document
        .get("floor")
        .and_then(|floor| floor.get("lines_percent"))
        .and_then(Value::as_f64)
        .expect("the baseline must record an overall floor");
    let measured_overall = document
        .get("measured")
        .and_then(|measured| measured.get("overall_lines_percent"))
        .and_then(Value::as_f64)
        .expect("the baseline must record an overall measurement");
    assert!(
        overall <= measured_overall,
        "the overall floor ({overall}) must not exceed the overall measurement ({measured_overall})"
    );
}

#[test]
fn the_ci_job_enforces_the_recorded_floor() {
    let document = baseline();
    let floor = document
        .get("floor")
        .and_then(|floor| floor.get("lines_percent"))
        .and_then(Value::as_f64)
        .expect("the baseline must record an overall floor");
    let expected = format!("--fail-under-lines {floor}");

    let workflow_path = repo_root().join(".github/workflows/rust-ci.yml");
    let workflow = fs::read_to_string(&workflow_path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", workflow_path.display()));

    assert!(
        workflow.contains("cargo llvm-cov"),
        "CI must run the coverage measurement, otherwise the floor is only a number in a file"
    );
    assert!(
        workflow.contains(&expected),
        "the coverage job must run at the recorded floor (`{expected}`); a different number here would let the two drift"
    );
    assert!(
        workflow.contains("--workspace"),
        "the coverage job must measure the whole workspace, not a subset"
    );
}
