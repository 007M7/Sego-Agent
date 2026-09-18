//! Contract conformance: schema ↔ implementation ↔ published examples.
//!
//! The three published contracts under `schema/` are the source of truth for
//! what Sego emits and what it accepts. This suite pins all three parties
//! together so drift fails the build instead of waiting for a consumer:
//!
//! 1. **Contract metadata** - `x-contract-id` / `x-contract-revision` /
//!    `x-contract-ratified` exist and match the recorded identity
//!    (`sego.review.artifact/v1` rev 2, `sego.review.index-entry` rev 1,
//!    `sego.sidecar.envelope` rev 1).
//! 2. **Implementation → schema** - a real artifact written through
//!    [`runtime::persist_review_artifact`] and the appended index line carry
//!    every required key, only declared keys, and only enum values the schema
//!    allows. The Rust enum labels must equal the schema enums exactly, in
//!    both directions.
//! 3. **Examples → schema** - every `json` block in the skill and the contract
//!    document is parsed and checked against the matching schema, which is
//!    the automated guard for the `context.user_intent` class of drift
//!    (DEV-DOC-02): a documented field the envelope never read, and now
//!    rejects.
//!
//! This is a structural conformance check, not a full Draft 2020-12
//! validator; it covers exactly the drift classes this repository has
//! actually produced (missing keys, undeclared keys, enum drift, stale
//! examples, contract-id drift).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use runtime::{
    persist_review_artifact, EvidenceStatus, ReviewFindingStatus, ReviewParseStatus, ReviewReport,
    ReviewScope, ReviewSeverity, ReviewTarget,
};
use serde_json::Value;

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is rust/crates/runtime; the schema/ and skills/
    // directories live at the repository root, three levels up.
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn schema(name: &str) -> Value {
    let path = repo_root().join("schema").join(format!("{name}.schema.json"));
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{} is not valid JSON: {error}", path.display()))
}

/// Resolve the object that actually holds `properties` for a schema root,
/// following a single top-level `$ref` (the index schema uses one).
fn root_object(schema: &Value) -> &Value {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let name = reference.rsplit('/').next().unwrap_or(reference);
        return schema
            .pointer(&format!("/$defs/{name}"))
            .unwrap_or_else(|| panic!("root $ref {reference} does not resolve"));
    }
    schema
}

fn properties_of(object: &Value) -> BTreeSet<String> {
    object
        .get("properties")
        .and_then(Value::as_object)
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default()
}

fn required_of(object: &Value) -> BTreeSet<String> {
    object
        .get("required")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default()
}

fn enum_of(schema: &Value, def: &str) -> BTreeSet<String> {
    let values = schema
        .pointer(&format!("/$defs/{def}/enum"))
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("$defs/{def}/enum is missing"));
    values
        .iter()
        .map(|value| {
            value.as_str().unwrap_or_else(|| panic!("{def} enum has a non-string entry")).to_owned()
        })
        .collect()
}

fn object_keys(value: &Value) -> BTreeSet<String> {
    value.as_object().map(|map| map.keys().cloned().collect()).unwrap_or_default()
}

fn temp_root(name: &str) -> PathBuf {
    let unique =
        SystemTime::now().duration_since(UNIX_EPOCH).expect("time should move forward").as_nanos();
    std::env::temp_dir().join(format!("sego-contract-{name}-{unique}"))
}

#[test]
fn contract_metadata_matches_the_recorded_identity() {
    let cases = [
        ("review-artifact", "sego.review.artifact/v1", 2),
        ("review-index-entry", "sego.review.index-entry", 1),
        ("sidecar-request-response", "sego.sidecar.envelope", 1),
    ];
    for (file, expected_id, expected_revision) in cases {
        let schema = schema(file);
        assert_eq!(
            schema["x-contract-id"].as_str(),
            Some(expected_id),
            "{file}: contract id drifted from the recorded identity"
        );
        assert_eq!(
            schema["x-contract-revision"].as_u64(),
            Some(expected_revision),
            "{file}: revision drifted; a change here is a contract change and must be coordinated"
        );
        let ratified = schema["x-contract-ratified"].as_str().unwrap_or_default();
        assert_eq!(ratified.len(), 10, "{file}: ratified must be a YYYY-MM-DD date");
        assert!(
            schema["$schema"].as_str().unwrap_or_default().contains("2020-12"),
            "{file}: must stay on Draft 2020-12"
        );
    }
}

fn sample_target_and_report() -> (ReviewTarget, ReviewReport) {
    let target = ReviewTarget {
        scope: ReviewScope::Staged,
        git_status: String::from("## main\nA  src/lib.rs\n"),
        staged_diff: String::from(
            "diff --git a/src/lib.rs b/src/lib.rs\n@@ -1,3 +1,4 @@\n old\n+new\n",
        ),
        unstaged_diff: String::new(),
        full_tree: String::new(),
        workspace_root: None,
    };
    let report = ReviewReport::from_model_output(
        r#"{"findings":[{"severity":"critical","file":"src/lib.rs","line":7,"title":"Credential leak","evidence":"diff adds a literal secret","risk":"secret exposure","suggestion":"remove the secret and load it from config","confidence":0.98,"verification_hint":"rg secret"}]}"#,
    );
    (target, report)
}

#[test]
fn persisted_artifact_conforms_to_the_published_schema() {
    let root = temp_root("artifact");
    let (target, report) = sample_target_and_report();
    let artifact = persist_review_artifact(&root, &target, &report)
        .expect("artifact should persist through the real writer");

    let json_text = fs::read_to_string(&artifact.json_path).expect("read persisted json");
    let artifact_json: Value =
        serde_json::from_str(&json_text).expect("persisted artifact is valid JSON");

    let schema = schema("review-artifact");
    let declared = properties_of(&schema);
    let required = required_of(&schema);
    let present = object_keys(&artifact_json);

    let missing: Vec<String> = required.difference(&present).cloned().collect();
    assert!(missing.is_empty(), "artifact is missing required keys: {missing:?}");

    let undeclared: Vec<String> = present.difference(&declared).cloned().collect();
    assert!(
        undeclared.is_empty(),
        "the writer emits keys the contract does not declare: {undeclared:?} - \
         add them to the schema (a contract change) or stop emitting them"
    );

    assert_eq!(artifact_json["schema_version"], 1);

    // The artifact id shape is contractual (AGENTS.md: review-<epoch>-<prefix>).
    let id = artifact_json["id"].as_str().expect("id is a string");
    let (prefix, epoch, hash) = match id.split_once("review-") {
        Some(("", rest)) => match rest.split_once('-') {
            Some((epoch, hash)) => ("", epoch, hash),
            None => panic!("artifact id {id} lacks the diff-prefix segment"),
        },
        _ => panic!("artifact id {id} does not start with 'review-'"),
    };
    let _ = prefix;
    assert!(epoch.chars().all(|c| c.is_ascii_digit()) && epoch.len() >= 9, "id epoch: {id}");
    assert_eq!(hash.len(), 12, "id diff-prefix must be 12 chars: {id}");
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()), "id diff-prefix must be hex: {id}");

    // Enum values on the artifact itself.
    let parse_status = enum_of(&schema, "parse_status");
    assert!(
        parse_status.contains(artifact_json["parse_status"].as_str().unwrap_or_default()),
        "parse_status drifted from the schema enum"
    );
    let severity = enum_of(&schema, "severity");
    for finding in artifact_json["findings"].as_array().into_iter().flatten() {
        let value = finding["severity"].as_str().unwrap_or_default();
        assert!(severity.contains(value), "finding severity {value} is not in the schema enum");
    }
    let evidence_status = enum_of(&schema, "evidence_status");
    if let Some(coverage) = artifact_json.get("evidence_coverage") {
        assert!(enum_of(&schema, "severity").len() >= 5, "severity enum must keep five values");
        let _ = coverage;
    }
    for finding in artifact_json["findings"].as_array().into_iter().flatten() {
        if let Some(status) = finding["evidence_status"].as_str() {
            assert!(
                evidence_status.contains(status),
                "evidence_status {status} is not in the schema enum"
            );
        }
    }

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn appended_index_entries_conform_to_the_published_schema() {
    let root = temp_root("index");
    let (target, report) = sample_target_and_report();
    let artifact = persist_review_artifact(&root, &target, &report)
        .expect("artifact should persist so an index line is appended");

    let index_text = fs::read_to_string(&artifact.index_path).expect("read index");
    let schema = schema("review-index-entry");
    let entry_object = root_object(&schema);
    let declared = properties_of(entry_object);
    let required = required_of(entry_object);
    let parse_status = enum_of(&schema, "parse_status");
    let severity = enum_of(&schema, "severity");

    let mut checked = 0_usize;
    for line in index_text.lines().filter(|line| !line.trim().is_empty()) {
        let entry: Value = serde_json::from_str(line)
            .unwrap_or_else(|error| panic!("index line is not JSON: {line} - {error}"));
        let present = object_keys(&entry);
        let missing: Vec<String> = required.difference(&present).cloned().collect();
        assert!(missing.is_empty(), "index entry missing required keys: {missing:?}");
        let undeclared: Vec<String> = present.difference(&declared).cloned().collect();
        assert!(undeclared.is_empty(), "index entry carries undeclared keys {undeclared:?}");
        assert!(
            parse_status.contains(entry["parse_status"].as_str().unwrap_or_default()),
            "index parse_status drifted"
        );
        if let Some(highest) = entry["highest_severity"].as_str() {
            assert!(severity.contains(highest), "index severity {highest} drifted");
        }
        checked += 1;
    }
    assert!(checked >= 1, "no index entries were produced to check");

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn rust_enums_equal_the_schema_enums_in_both_directions() {
    let artifact_schema = schema("review-artifact");
    let index_schema = schema("review-index-entry");

    let severities: Vec<&str> = ReviewSeverity::ALL_LABELS.to_vec();
    let expected_severity = enum_of(&artifact_schema, "severity");
    let actual_severity: BTreeSet<&str> = severities.iter().copied().collect();
    assert_eq!(
        actual_severity,
        expected_severity.iter().map(String::as_str).collect::<BTreeSet<_>>(),
        "severity: schema and implementation disagree"
    );

    let parse: BTreeSet<&str> = ReviewParseStatus::ALL_LABELS.iter().copied().collect();
    assert_eq!(
        parse,
        enum_of(&artifact_schema, "parse_status")
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        "parse_status: schema and implementation disagree"
    );

    let evidence: BTreeSet<&str> = EvidenceStatus::ALL_LABELS.iter().copied().collect();
    assert_eq!(
        evidence,
        enum_of(&artifact_schema, "evidence_status")
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        "evidence_status: schema and implementation disagree"
    );

    let finding_status: BTreeSet<&str> = ReviewFindingStatus::ALL_LABELS.iter().copied().collect();
    assert_eq!(
        finding_status,
        enum_of(&index_schema, "finding_status")
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        "finding_status: schema and implementation disagree"
    );

    // The envelope owns exactly one extra parse_status value.
    let sidecar = schema("sidecar-request-response");
    let mut envelope_only = enum_of(&artifact_schema, "parse_status");
    envelope_only.insert("no_diff".to_string());
    assert_eq!(
        enum_of(&sidecar, "parse_status"),
        envelope_only,
        "the sidecar envelope must be the artifact enum plus no_diff, nothing else"
    );
}

/// Extract every fenced `json` code block from a markdown file.
fn json_blocks(markdown_path: &Path) -> Vec<Value> {
    let text = fs::read_to_string(markdown_path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", markdown_path.display()));
    let mut blocks = Vec::new();
    let mut inside = false;
    let mut buffer = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if !inside && trimmed.starts_with("```") && trimmed[3..].trim_start().starts_with("json") {
            inside = true;
            buffer.clear();
            continue;
        }
        if inside && trimmed == "```" {
            inside = false;
            if let Ok(value) = serde_json::from_str::<Value>(&buffer) {
                blocks.push(value);
            } else {
                panic!(
                    "{} contains a ```json block that is not valid JSON:\n{buffer}",
                    markdown_path.display()
                );
            }
            continue;
        }
        if inside {
            buffer.push_str(line);
            buffer.push('\n');
        }
    }
    assert!(!inside, "{} has an unclosed ```json fence", markdown_path.display());
    blocks
}

#[test]
fn published_examples_only_use_fields_the_sidecar_contract_declares() {
    let skill = repo_root().join("skills").join("sego-review").join("SKILL.md");
    let sidecar = schema("sidecar-request-response");
    let request = sidecar.pointer("/$defs/request").expect("request def");
    let success = sidecar.pointer("/$defs/success_response").expect("success def");
    let request_props = properties_of(request);
    let context_props = properties_of(&request["properties"]["context"]);
    let success_props = properties_of(success);

    let blocks = json_blocks(&skill);
    assert!(!blocks.is_empty(), "the skill must keep at least one JSON example");

    let mut request_examples = 0;
    for block in &blocks {
        let keys = object_keys(block);
        if keys.contains("action") {
            request_examples += 1;
            let undeclared: Vec<String> = keys.difference(&request_props).cloned().collect();
            assert!(
                undeclared.is_empty(),
                "SKILL.md request example sends keys the envelope rejects: {undeclared:?}"
            );
            let action = block["action"].as_str().unwrap_or_default();
            let allowed = request
                .pointer("/properties/action/enum")
                .and_then(Value::as_array)
                .map(|list| list.iter().filter_map(Value::as_str).collect::<Vec<_>>())
                .unwrap_or_default();
            assert!(
                allowed.contains(&action),
                "SKILL.md request action {action} is not in the contract enum {allowed:?}"
            );
            if let Some(context) = block.get("context").and_then(Value::as_object) {
                let context_keys: BTreeSet<String> = context.keys().cloned().collect();
                let undeclared: Vec<String> =
                    context_keys.difference(&context_props).cloned().collect();
                assert!(
                    undeclared.is_empty(),
                    "SKILL.md context uses keys the envelope rejects: {undeclared:?} \
                     (this is the automated guard for the removed user_intent)"
                );
            }
        } else if keys.contains("status") {
            let undeclared: Vec<String> = keys.difference(&success_props).cloned().collect();
            assert!(
                undeclared.is_empty(),
                "SKILL.md response example shows keys the envelope never emits: {undeclared:?}"
            );
        }
    }
    assert!(
        request_examples >= 1,
        "the skill must keep a request example; it is the integration contract"
    );
}

#[test]
fn contract_document_examples_conform_to_the_artifact_schema() {
    let doc = repo_root().join("docs").join("REVIEW_ARTIFACT_CONTRACT.md");
    if !doc.exists() {
        return;
    }
    let schema = schema("review-artifact");
    let declared = properties_of(&schema);
    let required = required_of(&schema);
    let severity = enum_of(&schema, "severity");
    let parse_status = enum_of(&schema, "parse_status");

    let mut artifact_examples = 0;
    for block in json_blocks(&doc) {
        let keys = object_keys(&block);
        if !keys.contains("parse_status") {
            continue;
        }
        artifact_examples += 1;
        let missing: Vec<String> = required.difference(&keys).cloned().collect();
        assert!(
            missing.is_empty(),
            "REVIEW_ARTIFACT_CONTRACT.md artifact example is missing required keys: {missing:?}"
        );
        let undeclared: Vec<String> = keys.difference(&declared).cloned().collect();
        assert!(
            undeclared.is_empty(),
            "REVIEW_ARTIFACT_CONTRACT.md example uses undeclared keys: {undeclared:?}"
        );
        assert!(
            parse_status.contains(block["parse_status"].as_str().unwrap_or_default()),
            "example parse_status drifted from the schema enum"
        );
        for finding in block["findings"].as_array().into_iter().flatten() {
            assert!(
                severity.contains(finding["severity"].as_str().unwrap_or_default()),
                "example finding severity drifted from the schema enum"
            );
        }
    }
    assert!(
        artifact_examples >= 1,
        "REVIEW_ARTIFACT_CONTRACT.md must keep at least one full artifact example"
    );
}

// ---------------------------------------------------------------------------
// The completeness matrix (DEV-CON-07).
//
// `docs/REVIEW_ARTIFACT_CONTRACT.md` §8 states, per contract, which of twelve
// properties that contract actually carries. It exists because consumers were
// left to infer it, and the honest answer differs per contract. Being
// consumer-facing is exactly why it needs a check: a "declared" claim that the
// schema does not support is worse than no claim at all, because someone will
// build on it.
//
// The rule below is general rather than one assertion per cell: for every cell
// that claims a contract *declares* something, each backticked identifier in
// that claim must exist in that contract's schema. That is the drift that
// actually happens - a field renamed or removed while the prose stays put.
//
// It also asserts all twelve property rows exist, because a row quietly deleted
// is the other way this table stops being a completeness statement.
// ---------------------------------------------------------------------------

/// The twelve properties the completeness criterion names, in the order §8 uses.
const COMPLETENESS_PROPERTIES: [&str; 12] = [
    "Version",
    "Revision",
    "Stable id",
    "Provenance",
    "Idempotency",
    "Permission",
    "Audit",
    "Expiry",
    "Failure semantics",
    "Recovery",
    "Compatibility",
    "Deprecation window",
];

/// The three contracts, in the column order §8 uses.
const COMPLETENESS_CONTRACTS: [(&str, &str); 3] = [
    ("sego.review.artifact/v1", "review-artifact"),
    ("sego.sidecar.envelope", "sidecar-request-response"),
    ("sego.review.index-entry", "review-index-entry"),
];

/// Every key name a schema declares anywhere, including the contract metadata
/// that sits at the root rather than under `properties`.
fn declared_names(schema: &Value) -> BTreeSet<String> {
    fn walk(node: &Value, out: &mut BTreeSet<String>) {
        match node {
            Value::Object(map) => {
                for (key, value) in map {
                    out.insert(key.clone());
                    walk(value, out);
                }
            }
            Value::Array(items) => {
                for item in items {
                    walk(item, out);
                }
            }
            _ => {}
        }
    }
    let mut out = BTreeSet::new();
    walk(schema, &mut out);
    out
}

/// The `| Property | artifact | sidecar | index |` table from §8, keyed by
/// property name, as three cells.
fn completeness_matrix(document: &str) -> BTreeMap<String, Vec<String>> {
    let mut rows = BTreeMap::new();
    let mut inside = false;
    for line in document.lines() {
        if line.starts_with("## 8.") {
            inside = true;
            continue;
        }
        if inside && line.starts_with("## ") {
            break;
        }
        if !inside || !line.starts_with("| ") {
            continue;
        }
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() != 4 || cells[0] == "Property" || cells[0].starts_with("---") {
            continue;
        }
        rows.insert(cells[0].to_string(), cells[1..].iter().map(|c| (*c).to_string()).collect());
    }
    rows
}

fn backticked(text: &str) -> Vec<&str> {
    text.split('`').skip(1).step_by(2).collect()
}

#[test]
fn the_completeness_matrix_claims_only_what_the_schemas_declare() {
    let path = repo_root().join("docs/REVIEW_ARTIFACT_CONTRACT.md");
    let document = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    let matrix = completeness_matrix(&document);

    for property in COMPLETENESS_PROPERTIES {
        assert!(
            matrix.contains_key(property),
            "the completeness matrix must keep a row for `{property}`; a row removed is a claim withdrawn"
        );
    }

    for (index, (contract, file)) in COMPLETENESS_CONTRACTS.iter().enumerate() {
        let schema = schema(file);
        let declared = declared_names(&schema);

        for property in COMPLETENESS_PROPERTIES {
            let Some(cells) = matrix.get(property) else { continue };
            let cell = &cells[index];
            if !cell.starts_with("declared") {
                continue;
            }
            let named = backticked(cell);
            assert!(
                !named.is_empty(),
                "{contract}: the `{property}` row says \"declared\" without naming what declares it"
            );
            for name in named {
                assert!(
                    declared.contains(name),
                    "{contract} does not declare `{name}`, but the completeness matrix says it does \
                     (row `{property}`: {cell})"
                );
            }
        }
    }
}
