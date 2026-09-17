//! Dependency policy (DEV-SEC-14), enforced offline.
//!
//! The repository had no dependency audit: advisories, licences and dependency
//! *sources* were all unreviewed. Advisories and licences need the network
//! (`cargo-deny` fetches the RustSec database and crate metadata), so those run
//! in the `dependency-audit` CI job. This file covers the part that can be
//! decided from the repository alone, which means it is blocking on every
//! `cargo test --workspace` rather than only on a scheduled audit:
//!
//! 1. every resolved dependency comes from crates.io - no `git`, no alternate
//!    registry, no direct URL;
//! 2. no manifest declares a `git = "..."` dependency, so a git dependency is
//!    caught even before the lock file is regenerated;
//! 3. `deny.toml` still exists and still encodes the policy, so the audit
//!    configuration cannot be deleted or hollowed out without a test failure.

use std::fs;
use std::path::{Path, PathBuf};

const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index";

fn rust_dir() -> PathBuf {
    // rust/crates/runtime -> rust/
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn repo_root() -> PathBuf {
    rust_dir().join("..")
}

#[derive(Debug)]
struct LockedPackage {
    name: String,
    version: String,
    source: Option<String>,
}

/// Parse the `[[package]]` blocks of a lock file. Deliberately line-based: it
/// avoids a TOML dependency, and the lock format is generated, not authored.
fn locked_packages(lock: &str) -> Vec<LockedPackage> {
    let mut packages = Vec::new();
    let mut current: Option<LockedPackage> = None;
    for line in lock.lines() {
        if line.trim() == "[[package]]" {
            if let Some(package) = current.take() {
                packages.push(package);
            }
            current =
                Some(LockedPackage { name: String::new(), version: String::new(), source: None });
            continue;
        }
        let Some(package) = current.as_mut() else {
            continue;
        };
        let Some((key, value)) = line.split_once(" = ") else {
            continue;
        };
        let value = value.trim().trim_matches('"');
        match key.trim() {
            "name" => package.name = value.to_string(),
            "version" => package.version = value.to_string(),
            "source" => package.source = Some(value.to_string()),
            _ => {}
        }
    }
    if let Some(package) = current.take() {
        packages.push(package);
    }
    packages
}

fn rust_manifests() -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![rust_dir()];
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
            } else if path.file_name().is_some_and(|name| name == "Cargo.toml") {
                found.push(path);
            }
        }
    }
    found
}

/// Dependencies that do not resolve to crates.io, as `name version -> source`.
///
/// Split out from the test so the detector can be exercised against a lock that
/// actually contains an offender; asserting it on the real lock alone would
/// prove nothing about whether it can detect anything.
fn crates_io_offenders(lock: &str) -> Vec<String> {
    let mut offenders = Vec::new();
    for package in locked_packages(lock) {
        if let Some(source) = package.source {
            if source != CRATES_IO {
                offenders.push(format!("{} {} -> {source}", package.name, package.version));
            }
        }
    }
    offenders
}

/// `git = "..."` declarations in a manifest, as `line: text`.
fn git_declarations(manifest: &str) -> Vec<String> {
    manifest
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            let trimmed = line.trim_start();
            !trimmed.starts_with('#') && (trimmed.contains("git = ") || trimmed.contains("git="))
        })
        .map(|(index, line)| format!("{}: {}", index + 1, line.trim()))
        .collect()
}

#[test]
fn every_resolved_dependency_comes_from_crates_io() {
    let lock_path = rust_dir().join("Cargo.lock");
    let lock = fs::read_to_string(&lock_path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", lock_path.display()));
    let packages = locked_packages(&lock);

    assert!(
        packages.len() > 100,
        "parsed only {} packages; the lock scan is broken and this check would be vacuous",
        packages.len()
    );
    assert!(
        packages.iter().any(|package| package.source.is_some()),
        "no package carried a source at all; the lock format changed and the scan needs updating"
    );

    let offenders = crates_io_offenders(&lock);
    assert!(
        offenders.is_empty(),
        "a dependency does not come from crates.io; that is a supply-chain change and needs an \
         explicit decision (and an update to deny.toml):\n  {}",
        offenders.join("\n  ")
    );

    // The workspace's own crates are the only packages without a source, and
    // every one of them is named.
    let local: Vec<&LockedPackage> =
        packages.iter().filter(|package| package.source.is_none()).collect();
    assert!(!local.is_empty(), "the workspace should have source-less members");
    assert!(local.iter().all(|package| !package.name.is_empty()));
}

#[test]
fn a_git_or_alternate_registry_dependency_is_detected() {
    // Negative control for the scan above: a lock that does contain offenders
    // must be reported, by name, so the real-lock assertion is not passing by
    // accident.
    let synthetic = r#"
[[package]]
name = "trusted"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "from-git"
version = "0.2.0"
source = "git+https://github.com/example/from-git?rev=abc#abc"

[[package]]
name = "from-elsewhere"
version = "3.1.4"
source = "registry+https://internal.example/registry"

[[package]]
name = "workspace-member"
version = "0.1.9"
"#;
    let offenders = crates_io_offenders(synthetic);
    assert_eq!(offenders.len(), 2, "expected two offenders, got {offenders:?}");
    assert!(offenders.iter().any(|entry| entry.contains("from-git")), "{offenders:?}");
    assert!(offenders.iter().any(|entry| entry.contains("from-elsewhere")), "{offenders:?}");
    assert!(
        !offenders
            .iter()
            .any(|entry| entry.contains("trusted") || entry.contains("workspace-member")),
        "crates.io and path dependencies must not be reported: {offenders:?}"
    );
}

#[test]
fn no_manifest_declares_a_git_dependency() {
    let mut offenders = Vec::new();
    for manifest in rust_manifests() {
        let Ok(text) = fs::read_to_string(&manifest) else {
            continue;
        };
        for entry in git_declarations(&text) {
            offenders.push(format!("{}: {}", manifest.display(), entry));
        }
    }
    assert!(
        offenders.is_empty(),
        "a git dependency bypasses the registry and the checksum trail; argue for it explicitly \
         first:\n  {}",
        offenders.join("\n  ")
    );

    // And the detector itself has to work: a manifest that does declare one is
    // reported, including the commented-out case that must not count.
    let synthetic = "serde = { version = \"1\" }\n# git = \"https://example.invalid\"\nleft = { git = \"https://github.com/example/left\" }\n";
    let detected = git_declarations(synthetic);
    assert_eq!(detected.len(), 1, "expected exactly the live declaration, got {detected:?}");
    assert!(detected[0].contains("left"), "{detected:?}");
}

#[test]
fn the_dependency_audit_policy_is_present_and_not_hollowed_out() {
    let path = rust_dir().join("deny.toml");
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));

    for section in ["[advisories]", "[licenses]", "[sources]", "[bans]"] {
        assert!(text.contains(section), "deny.toml must keep its {section} section");
    }
    // An explicit, empty ignore list is the baseline mechanism: waivers are
    // listed one per line with a reason, rather than by disabling the check.
    assert!(
        text.contains("ignore = []")
            || text.contains("ignore = [\n")
            || text.contains("ignore = [ {"),
        "deny.toml must carry an explicit ignore list so waivers stay visible and reviewable"
    );
    assert!(
        text.contains("unknown-git = \"deny\"") && text.contains("unknown-registry = \"deny\""),
        "dependency sources must stay denied, matching the offline test beside this one"
    );
    assert!(
        text.contains("allow-registry = [\"https://github.com/rust-lang/crates.io-index\"]"),
        "the allowed registry must be named explicitly rather than left open"
    );
    assert!(
        text.contains("allow = ["),
        "the licence allow list must stay explicit rather than defaulting to permissive"
    );
    // A waiver is only a waiver if it says why. The comment above the list has
    // always claimed "one reason per entry"; now the claim is checked, so an
    // entry added in a hurry cannot quietly suppress an advisory.
    let waivers = text
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("{ id = \"RUSTSEC-"))
        .collect::<Vec<_>>();
    for waiver in &waivers {
        assert!(
            waiver.contains("reason = \"") && !waiver.contains("reason = \"\""),
            "every advisory waiver must carry a reason: {waiver}"
        );
    }
    // The path-wildcard exemption is a deliberate, narrow decision about
    // in-repo dependencies. Naming it here means removing or widening it takes
    // an edit to this test as well.
    assert!(
        text.contains("allow-wildcard-paths = true"),
        "the wildcard policy exempts in-repo path dependencies explicitly; if that changed, update the reasoning rather than dropping the line"
    );
    assert!(
        text.contains("wildcards = \"deny\""),
        "registry wildcards must stay denied - the exemption is for path dependencies only"
    );
    // The policy belongs to the workspace it audits.
    assert!(repo_root().join("rust/Cargo.toml").exists());
}
