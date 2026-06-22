use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NlIntent {
    WorkspaceShow,
    WorkspaceSwitch { path: String },
    Review { scope: Option<String> },
    ReviewSafety { staged: bool },
    ExportLastResponse { path: Option<String> },
    ExportSession { path: Option<String> },
    UpdateCheck,
    UpdateApply,
    Exit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NlIntentMiss {
    NeedsMoreDetail { action: &'static str, example: &'static str },
}

pub fn parse_nl_intent(input: &str) -> Option<NlIntent> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return None;
    }
    let normalized = strip_polite_prefix(trimmed);

    parse_workspace_intent(normalized)
        .or_else(|| parse_export_last_response_intent(normalized))
        .or_else(|| parse_export_session_intent(normalized))
        .or_else(|| parse_review_intent(normalized))
        .or_else(|| parse_update_intent(normalized))
        .or_else(|| parse_exit_intent(normalized))
}

pub fn classify_nl_intent_miss(input: &str) -> Option<NlIntentMiss> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') || parse_nl_intent(trimmed).is_some() {
        return None;
    }
    let normalized = strip_polite_prefix(trimmed);
    let lower = normalized.to_ascii_lowercase();
    if is_generation_request(normalized, &lower)
        || has_explanation_verb(normalized, &lower)
        || has_how_to_question(normalized, &lower)
    {
        return None;
    }

    if mentions_workspace_switch_without_path(normalized, &lower) {
        return Some(NlIntentMiss::NeedsMoreDetail {
            action: "切换工作区",
            example: "切换到 D:\\YourProject",
        });
    }
    if mentions_export_without_target(normalized, &lower) {
        return Some(NlIntentMiss::NeedsMoreDetail {
            action: "导出/保存",
            example: "把刚才的审查结果写成 E:\\code\\review.md",
        });
    }
    if mentions_update_without_target(normalized, &lower) {
        return Some(NlIntentMiss::NeedsMoreDetail {
            action: "更新",
            example: "检查更新 / 更新到最新版",
        });
    }
    if mentions_review_without_scope(normalized, &lower) {
        return Some(NlIntentMiss::NeedsMoreDetail {
            action: "代码审查",
            example: "帮我 review 当前改动 / 审查整个项目代码",
        });
    }

    None
}

fn parse_workspace_intent(input: &str) -> Option<NlIntent> {
    let lower = input.to_ascii_lowercase();
    if has_explanation_verb(input, &lower) || has_how_to_question(input, &lower) {
        return None;
    }

    if matches!(
        lower.as_str(),
        "pwd" | "cwd" | "where am i" | "show workspace" | "current workspace" | "current directory"
    ) || is_workspace_show_phrase(input, &lower)
    {
        return Some(NlIntent::WorkspaceShow);
    }

    for prefix in [
        "切换到",
        "切到",
        "切换工作区到",
        "切换工作目录到",
        "打开项目",
        "进入项目",
        "进入目录",
        "把工作目录切到",
        "change directory to",
        "switch workspace to",
        "switch to workspace",
        "open project",
        "use workspace",
    ] {
        if let Some(path) = strip_intent_prefix(input, prefix) {
            if looks_like_workspace_path(path) {
                return Some(NlIntent::WorkspaceSwitch { path: path.to_string() });
            }
        }
    }

    if let Some(path) = strip_intent_prefix(input, "cd ") {
        if looks_like_workspace_path(path) {
            return Some(NlIntent::WorkspaceSwitch { path: path.to_string() });
        }
    }

    None
}

fn parse_export_last_response_intent(input: &str) -> Option<NlIntent> {
    let lower = input.to_ascii_lowercase();
    if is_generation_request(input, &lower)
        || has_explanation_verb(input, &lower)
        || has_how_to_question(input, &lower)
    {
        return None;
    }

    let has_export_verb = lower.contains("export")
        || lower.contains("save")
        || lower.contains("write")
        || input.contains("导出")
        || input.contains("保存")
        || input.contains("写入")
        || input.contains("写成")
        || input.contains("落盘");
    if !has_export_verb {
        return None;
    }

    // C20.5-B: must contain explicit "last/previous/刚才/上一条" semantic.
    // "save review report" alone is NOT sufficient.
    let targets_last_response = contains_english_previous_response_marker(&lower)
        || input.contains("刚才")
        || input.contains("上一条")
        || input.contains("上一次")
        || input.contains("上回");
    if !targets_last_response {
        return None;
    }

    let path = extract_export_path(input);
    if path.is_none()
        && !(lower.contains(".md") || lower.contains("markdown") || input.contains("md"))
    {
        return None;
    }

    Some(NlIntent::ExportLastResponse { path })
}

fn contains_english_previous_response_marker(lower: &str) -> bool {
    [
        "last response",
        "previous response",
        "latest response",
        "last reply",
        "previous reply",
        "last review",
        "previous review",
        "last result",
        "previous result",
        "last assistant",
        "previous assistant",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn parse_export_session_intent(input: &str) -> Option<NlIntent> {
    let lower = input.to_ascii_lowercase();
    if is_generation_request(input, &lower)
        || has_explanation_verb(input, &lower)
        || has_how_to_question(input, &lower)
    {
        return None;
    }

    let has_export_verb = lower.contains("export")
        || lower.contains("save")
        || input.contains("导出")
        || input.contains("保存")
        || input.contains("备份");
    let targets_session = lower.contains("session")
        || lower.contains("conversation")
        || lower.contains("transcript")
        || input.contains("会话")
        || input.contains("对话")
        || input.contains("聊天记录");
    if has_export_verb && targets_session {
        return Some(NlIntent::ExportSession { path: extract_export_path(input) });
    }

    None
}

fn parse_review_intent(input: &str) -> Option<NlIntent> {
    let lower = input.to_ascii_lowercase();
    if is_generation_request(input, &lower)
        || has_explanation_verb(input, &lower)
        || has_how_to_question(input, &lower)
    {
        return None;
    }

    let has_review_verb = lower.contains("review")
        || lower.contains("audit")
        || lower.contains("check")
        || input.contains("审查")
        || input.contains("审核")
        || input.contains("检查");
    if !has_review_verb {
        return None;
    }

    let mentions_code_or_changes = lower.contains("code")
        || lower.contains("diff")
        || lower.contains("changes")
        || lower.contains("project")
        || lower.contains("workspace")
        || input.contains("代码")
        || input.contains("改动")
        || input.contains("修改")
        || input.contains("项目")
        || input.contains("工作区");
    let mentions_safety = lower.contains("safety")
        || lower.contains("security")
        || lower.contains("vulnerability")
        || input.contains("安全")
        || input.contains("漏洞")
        || input.contains("风险")
        || input.contains("新手安全");

    if mentions_safety {
        return Some(NlIntent::ReviewSafety {
            staged: lower.contains("staged") || input.contains("暂存") || input.contains("已暂存"),
        });
    }

    if !mentions_code_or_changes {
        return None;
    }

    Some(NlIntent::Review { scope: review_scope_hint(input, &lower) })
}

fn parse_update_intent(input: &str) -> Option<NlIntent> {
    let lower = input.to_ascii_lowercase();
    if is_generation_request(input, &lower)
        || has_explanation_verb(input, &lower)
        || has_how_to_question(input, &lower)
    {
        return None;
    }

    let mentions_sego = lower.contains("sego") || input.contains("色狗") || input.contains("色鬼");
    let check_update = (mentions_sego
        && (lower.contains("check update")
            || lower.contains("check for updates")
            || lower.contains("latest version")))
        || matches!(lower.as_str(), "check sego update" | "check sego updates")
        || input.contains("检查更新")
        || input.contains("有没有新版")
        || (mentions_sego && input.contains("最新版本"));
    if check_update {
        return Some(NlIntent::UpdateCheck);
    }

    let apply_update = lower == "update"
        || lower == "upgrade"
        || (mentions_sego && (lower.contains("update") || lower.contains("upgrade")))
        || input.contains("更新到最新版")
        || input.contains("升级到最新版")
        || input.contains("帮我升级")
        || input.contains("更新 Sego")
        || input.contains("升级 Sego")
        || input.contains("更新sego")
        || input.contains("升级sego");
    if apply_update {
        return Some(NlIntent::UpdateApply);
    }

    None
}

fn parse_exit_intent(input: &str) -> Option<NlIntent> {
    let lower = input.to_ascii_lowercase();
    if is_generation_request(input, &lower)
        || has_explanation_verb(input, &lower)
        || has_how_to_question(input, &lower)
    {
        return None;
    }

    if matches!(lower.as_str(), "exit" | "quit" | "bye" | "close sego")
        || matches!(input, "退出" | "退出 Sego" | "退出sego" | "关闭 Sego" | "关闭sego")
        || input == "结束会话"
    {
        return Some(NlIntent::Exit);
    }

    None
}

fn review_scope_hint(input: &str, lower: &str) -> Option<String> {
    if lower.contains("staged") || input.contains("暂存") || input.contains("已暂存") {
        return Some("staged".to_string());
    }
    if lower.contains("unstaged") || lower.contains("working tree") || input.contains("未暂存") {
        return Some("unstaged".to_string());
    }
    if lower.contains("workspace")
        || lower.contains("project")
        || lower.contains("all")
        || input.contains("整个项目")
        || input.contains("全项目")
        || input.contains("工作区")
    {
        return Some("workspace".to_string());
    }
    None
}

fn strip_polite_prefix(input: &str) -> &str {
    let mut current = input.trim();
    for prefix in ["请帮我", "麻烦你", "麻烦", "帮我", "请", "please "] {
        if let Some(rest) = strip_intent_prefix(current, prefix) {
            current = rest.trim();
            break;
        }
    }
    current
}

fn is_workspace_show_phrase(input: &str, lower: &str) -> bool {
    matches!(
        input,
        "当前目录"
            | "当前工作目录"
            | "工作目录"
            | "当前工作区"
            | "现在在哪个目录"
            | "现在工作目录"
            | "现在工作区"
            | "看当前工作区"
            | "看工作区"
            | "显示当前工作区"
            | "查看当前工作区"
            | "显示工作目录"
            | "查看工作目录"
    ) || matches!(lower, "show cwd" | "show current directory" | "show current workspace")
}

fn is_generation_request(input: &str, lower: &str) -> bool {
    lower.starts_with("write ")
        || lower.starts_with("create ")
        || lower.starts_with("implement ")
        || lower.starts_with("generate ")
        || input.starts_with("写一个")
        || input.starts_with("写个")
        || input.starts_with("实现")
        || input.starts_with("生成一个")
        || input.starts_with("生成一份")
        || input.starts_with("开发一个")
        || input.starts_with("做一个")
}

fn has_explanation_verb(input: &str, lower: &str) -> bool {
    lower.contains("explain")
        || lower.contains("what is")
        || lower.contains("how does")
        || input.contains("解释")
        || input.contains("说明")
        || input.contains("什么是")
        || input.contains("是什么意思")
        || input.contains("什么意思")
}

fn has_how_to_question(input: &str, lower: &str) -> bool {
    lower.contains("how to")
        || lower.contains("how do i")
        || input.contains("如何")
        || input.contains("怎么")
        || input.contains("怎样")
}

fn mentions_workspace_switch_without_path(input: &str, lower: &str) -> bool {
    matches!(lower, "cd" | "switch workspace" | "change directory")
        || matches!(input, "切换目录" | "切换工作区" | "切换工作目录" | "打开项目" | "进入目录")
}

fn mentions_export_without_target(input: &str, lower: &str) -> bool {
    // C20.5-B: catch fuzzy save/export intents that lack explicit target.
    // Provide /dir guidance instead of silently routing to model.
    matches!(
        lower,
        "export"
            | "save report"
            | "save review"
            | "export report"
            | "save as markdown"
            | "save as md"
            | "export as markdown"
            | "export as md"
            | "write review"
            | "save result"
            | "export result"
    ) || matches!(
        input,
        "导出"
            | "保存报告"
            | "导出报告"
            | "保存审查结果"
            | "导出审查结果"
            | "保存审查报告"
            | "导出审查报告"
            | "保存为md"
            | "保存为markdown"
            | "导出为md"
            | "导出为markdown"
            | "写成md"
            | "写成markdown"
            | "输出审查报告"
            | "输出报告"
            | "输出为md"
            | "生成报告"
            | "保存全部"
            | "导出全部"
            | "保存所有"
            | "导出所有"
    )
}

fn mentions_update_without_target(input: &str, lower: &str) -> bool {
    matches!(lower, "update it" | "upgrade it")
        || matches!(input, "更新一下" | "升级一下" | "帮我更新" | "帮我升级")
}

fn mentions_review_without_scope(input: &str, lower: &str) -> bool {
    matches!(lower, "review" | "code review" | "audit" | "check code")
        || matches!(input, "审查" | "审核" | "检查代码" | "代码审查")
}

fn extract_export_path(input: &str) -> Option<String> {
    for quoted in extract_quoted_segments(input) {
        if looks_like_export_path(&quoted) {
            return Some(quoted);
        }
    }

    input
        .split_whitespace()
        .rev()
        .map(|part| {
            part.trim_matches(|ch: char| {
                matches!(ch, '"' | '\'' | '`' | '。' | '，' | ',' | ';' | '；' | ':' | '：')
            })
        })
        .find(|part| looks_like_export_path(part))
        .map(ToOwned::to_owned)
}

fn extract_quoted_segments(input: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for ch in input.chars() {
        match quote {
            Some(end) if ch == end => {
                let value = current.trim();
                if !value.is_empty() {
                    segments.push(value.to_string());
                }
                current.clear();
                quote = None;
            }
            Some(_) => current.push(ch),
            None if matches!(ch, '"' | '\'' | '`' | '“' | '‘') => {
                quote = Some(match ch {
                    '“' => '”',
                    '‘' => '’',
                    other => other,
                });
            }
            None => {}
        }
    }
    segments
}

fn looks_like_export_path(value: &str) -> bool {
    let path = Path::new(value);
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"))
        || value.contains('\\')
        || value.contains('/')
}

fn looks_like_workspace_path(value: &str) -> bool {
    let trimmed = value.trim().trim_matches('"');
    if trimmed.is_empty()
        || has_sentence_punctuation(trimmed)
        || trimmed.contains("是什么")
        || trimmed.contains("什么意思")
        || trimmed.contains("如何")
        || trimmed.contains("怎么")
    {
        return false;
    }
    trimmed == "."
        || trimmed.starts_with("..")
        || trimmed.contains('\\')
        || trimmed.contains('/')
        || trimmed.chars().nth(1) == Some(':')
        || looks_like_simple_relative_path(trimmed)
}

fn has_sentence_punctuation(value: &str) -> bool {
    value.chars().any(|ch| matches!(ch, '?' | '？' | '。' | '！' | '!'))
}

fn looks_like_simple_relative_path(value: &str) -> bool {
    !value.chars().any(char::is_whitespace)
        && value.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        && value.chars().any(|ch| ch.is_ascii_alphanumeric())
}

fn strip_intent_prefix<'a>(input: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_ascii() {
        let lower = input.to_ascii_lowercase();
        let lower_prefix = prefix.to_ascii_lowercase();
        if lower.starts_with(&lower_prefix) {
            let value = input[prefix.len()..].trim().trim_matches('"');
            return (!value.is_empty()).then_some(value);
        }
        return None;
    }
    input
        .strip_prefix(prefix)
        .map(|path| path.trim().trim_matches('"'))
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::{
        classify_nl_intent_miss, contains_english_previous_response_marker, parse_nl_intent,
        NlIntent, NlIntentMiss,
    };

    #[test]
    fn parses_workspace_show_intents() {
        assert_eq!(parse_nl_intent("当前工作区"), Some(NlIntent::WorkspaceShow));
        assert_eq!(parse_nl_intent("请帮我看当前工作区"), Some(NlIntent::WorkspaceShow));
        assert_eq!(parse_nl_intent("where am i"), Some(NlIntent::WorkspaceShow));
    }

    #[test]
    fn parses_workspace_switch_intents_with_windows_paths() {
        assert_eq!(
            parse_nl_intent("切换到 D:\\Project"),
            Some(NlIntent::WorkspaceSwitch { path: "D:\\Project".to_string() })
        );
        assert_eq!(
            parse_nl_intent("打开项目 \"E:\\中文 项目\""),
            Some(NlIntent::WorkspaceSwitch { path: "E:\\中文 项目".to_string() })
        );
        assert_eq!(
            parse_nl_intent("cd ../demo"),
            Some(NlIntent::WorkspaceSwitch { path: "../demo".to_string() })
        );
        assert_eq!(
            parse_nl_intent("切到 sego-demo"),
            Some(NlIntent::WorkspaceSwitch { path: "sego-demo".to_string() })
        );
    }

    #[test]
    fn keeps_ambiguous_cd_text_for_the_model() {
        assert_eq!(parse_nl_intent("cd command 是什么意思"), None);
        assert_eq!(parse_nl_intent("cd should be documented in README"), None);
        assert_eq!(parse_nl_intent("切换到哪个目录更合理？"), None);
        assert_eq!(parse_nl_intent("工作目录是什么意思"), None);
    }

    #[test]
    fn parses_export_last_response_intents() {
        assert_eq!(
            parse_nl_intent("把刚才的审查结果写成 E:\\code\\PR43-review.md"),
            Some(NlIntent::ExportLastResponse {
                path: Some("E:\\code\\PR43-review.md".to_string())
            })
        );
        assert_eq!(
            parse_nl_intent("save the last review report to PR43-review.md"),
            Some(NlIntent::ExportLastResponse { path: Some("PR43-review.md".to_string()) })
        );
        assert_eq!(
            parse_nl_intent("保存刚才的结果到 \"E:\\code\\PR43 review.md\""),
            Some(NlIntent::ExportLastResponse {
                path: Some("E:\\code\\PR43 review.md".to_string())
            })
        );
    }

    #[test]
    fn export_last_response_requires_explicit_previous_response_target() {
        assert_eq!(
            parse_nl_intent("审查当前改动，完成后输出结论"),
            Some(NlIntent::Review { scope: None })
        );
        assert_eq!(
            parse_nl_intent(
                "请审查 E:\\Sego\\source 当前 C16 最终改动，审查完成后输出结论和阻塞问题"
            ),
            Some(NlIntent::Review { scope: None })
        );
        // C20.5-A: bare "save review report" without "last" does NOT trigger export
        // (preserves C16 safe boundary).
        assert_eq!(parse_nl_intent("save review report to PR43-review.md"), None);
        assert_eq!(
            parse_nl_intent("save the last review report to PR43-review.md"),
            Some(NlIntent::ExportLastResponse { path: Some("PR43-review.md".to_string()) })
        );
        assert_eq!(
            parse_nl_intent("export the latest response to PR43-review.md"),
            Some(NlIntent::ExportLastResponse { path: Some("PR43-review.md".to_string()) })
        );
        // These filenames contain "last" as a substring, but they are not previous-response
        // references. If a catch-all `contains("last")` returns, these tests should fail.
        assert_eq!(parse_nl_intent("save blast.md"), None,);
        assert_eq!(parse_nl_intent("save last_report.md"), None,);
        assert_eq!(parse_nl_intent("what was the last review for this module?"), None);
        assert_eq!(parse_nl_intent("what was the last review for this module"), None);
    }

    #[test]
    fn english_previous_response_marker_is_conservative() {
        for input in [
            "last response",
            "previous response",
            "latest response",
            "last reply",
            "previous reply",
            "last review",
            "previous review",
            "last result",
            "previous result",
            "last assistant",
            "previous assistant",
        ] {
            assert!(contains_english_previous_response_marker(input), "{input}");
        }

        for input in [
            "",
            " ",
            "last",
            "previous",
            "latest",
            "the last one",
            "review last",
            "what was last",
            "blast.md",
            "last_report.md",
            "last-response.md",
        ] {
            assert!(!contains_english_previous_response_marker(input), "{input}");
        }
    }

    #[test]
    fn parses_export_session_intents() {
        assert_eq!(
            parse_nl_intent("导出当前会话到 E:\\code\\session.md"),
            Some(NlIntent::ExportSession { path: Some("E:\\code\\session.md".to_string()) })
        );
        assert_eq!(
            parse_nl_intent("save this conversation"),
            Some(NlIntent::ExportSession { path: None })
        );
    }

    #[test]
    fn parses_review_intents() {
        assert_eq!(parse_nl_intent("帮我 review 当前改动"), Some(NlIntent::Review { scope: None }));
        assert_eq!(parse_nl_intent("帮我review当前改动"), Some(NlIntent::Review { scope: None }));
        assert_eq!(parse_nl_intent("请审查当前修改"), Some(NlIntent::Review { scope: None }));
        assert_eq!(parse_nl_intent("检查当前改动"), Some(NlIntent::Review { scope: None }));
        assert_eq!(parse_nl_intent("审查当前改动"), Some(NlIntent::Review { scope: None }));
        assert_eq!(
            parse_nl_intent("审查整个项目代码"),
            Some(NlIntent::Review { scope: Some("workspace".to_string()) })
        );
        assert_eq!(
            parse_nl_intent("review staged changes"),
            Some(NlIntent::Review { scope: Some("staged".to_string()) })
        );
        assert_eq!(
            parse_nl_intent("检查已暂存代码"),
            Some(NlIntent::Review { scope: Some("staged".to_string()) })
        );
    }

    #[test]
    fn parses_safety_review_intents() {
        assert_eq!(parse_nl_intent("检查安全问题"), Some(NlIntent::ReviewSafety { staged: false }));
        assert_eq!(
            parse_nl_intent("检查刚实现的代码安全问题"),
            Some(NlIntent::ReviewSafety { staged: false })
        );
        assert_eq!(
            parse_nl_intent("检查已暂存代码的安全风险"),
            Some(NlIntent::ReviewSafety { staged: true })
        );
    }

    #[test]
    fn parses_update_and_exit_intents() {
        assert_eq!(parse_nl_intent("检查更新"), Some(NlIntent::UpdateCheck));
        assert_eq!(parse_nl_intent("check Sego updates"), Some(NlIntent::UpdateCheck));
        assert_eq!(parse_nl_intent("更新到最新版"), Some(NlIntent::UpdateApply));
        assert_eq!(parse_nl_intent("退出"), Some(NlIntent::Exit));
    }

    #[test]
    fn does_not_intercept_generation_or_explanation_requests() {
        assert_eq!(parse_nl_intent("帮我写一个 markdown parser"), None);
        assert_eq!(parse_nl_intent("解释一下 update 函数"), None);
        assert_eq!(parse_nl_intent("实现 review 页面"), None);
        assert_eq!(parse_nl_intent("create a code review checklist"), None);
        assert_eq!(parse_nl_intent("怎么更新 Sego"), None);
        assert_eq!(parse_nl_intent("check for updates in this changelog"), None);
        assert_eq!(parse_nl_intent("写一份 review 报告模板"), None);
    }

    #[test]
    fn ignores_slash_commands() {
        assert_eq!(parse_nl_intent("/review staged"), None);
        assert_eq!(parse_nl_intent("/cd D:\\Project"), None);
    }

    #[test]
    fn classifies_only_high_confidence_local_action_misses() {
        assert_eq!(
            classify_nl_intent_miss("保存报告"),
            Some(NlIntentMiss::NeedsMoreDetail {
                action: "导出/保存",
                example: "把刚才的审查结果写成 E:\\code\\review.md"
            })
        );
        for input in ["save as md", "export as markdown", "保存为md", "输出审查报告"] {
            assert_eq!(
                classify_nl_intent_miss(input),
                Some(NlIntentMiss::NeedsMoreDetail {
                    action: "导出/保存",
                    example: "把刚才的审查结果写成 E:\\code\\review.md"
                }),
                "{input}"
            );
        }
        for input in ["导出全部", "保存所有"] {
            assert_eq!(
                classify_nl_intent_miss(input),
                Some(NlIntentMiss::NeedsMoreDetail {
                    action: "导出/保存",
                    example: "把刚才的审查结果写成 E:\\code\\review.md"
                }),
                "{input}"
            );
        }
        assert_eq!(classify_nl_intent_miss("把刚才的审查结果写成 E:\\code\\review.md"), None);
        assert_eq!(classify_nl_intent_miss("save the last review report to PR43-review.md"), None);
        assert_eq!(classify_nl_intent_miss("查看日志"), None);
        assert_eq!(classify_nl_intent_miss("清理日志"), None);
        assert_eq!(classify_nl_intent_miss("显示日志"), None);
        assert_eq!(
            classify_nl_intent_miss("切换目录"),
            Some(NlIntentMiss::NeedsMoreDetail {
                action: "切换工作区",
                example: "切换到 D:\\YourProject"
            })
        );
        assert_eq!(classify_nl_intent_miss("帮我写一个 review 报告模板"), None);
        assert_eq!(classify_nl_intent_miss("怎么保存报告"), None);
        assert_eq!(classify_nl_intent_miss("帮我 review 当前改动"), None);
    }
}
