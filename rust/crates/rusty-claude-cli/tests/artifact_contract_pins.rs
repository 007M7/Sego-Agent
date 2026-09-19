//! Contract pins that span two crates (`SEG-ADR-004`).
//!
//! The review artifact's egress classification counts calls to one tool name. It
//! reads that name from a constant in `runtime`, while the tool itself is
//! registered in `tools` — a crate `runtime` does not depend on, so nothing else
//! would notice a rename on either side. The consequence of a silent rename is
//! specific and one-directional: the fetch count becomes zero, and every review is
//! recorded as having fetched nothing, which under-reports egress.

#[test]
fn the_tool_name_the_artifact_classifies_by_is_a_registered_tool() {
    let name = runtime::code_review::WEB_FETCH_TOOL_NAME;
    // A loopback address is refused by the tool's own host policy, so this makes no
    // network call. Whatever the outcome - a policy refusal or a connection error -
    // it must not be "that tool does not exist".
    let input = serde_json::json!({ "url": "http://127.0.0.1:9/", "prompt": "pin probe" });
    if let Err(message) = tools::execute_tool(name, &input) {
        assert!(
            !message.contains("unsupported tool"),
            "`{name}` is no longer a registered tool name, so the artifact's fetch \
             count would silently become zero: {message}"
        );
    }
}
