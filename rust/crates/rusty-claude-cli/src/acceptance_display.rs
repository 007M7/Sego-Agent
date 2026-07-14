use runtime::{
    AcceptanceReasonCode, AcceptanceRecord, AcceptanceState, NextActionCode, RemediationStatus,
    ReviewKind, ReviewSeverity, UnresolvedFinding,
};
use serde::Serialize;

use crate::review_card::escape_html;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceptanceLocale {
    ZhCn,
    En,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AcceptanceTaskBox {
    pub title: String,
    pub state_label: String,
    pub node_review_count: usize,
    pub task_end_review_count: usize,
    pub full_review_count: usize,
    pub automatically_remediated_count: usize,
    pub unresolved_count: usize,
    pub primary_reason: String,
    pub next_action: String,
    pub details_label: String,
    pub evidence_label: String,
}

#[must_use]
pub fn task_box_for(record: &AcceptanceRecord, locale: AcceptanceLocale) -> AcceptanceTaskBox {
    let (node_review_count, task_end_review_count, full_review_count) = review_counts(record);
    let automatically_remediated_count = record
        .review_events
        .iter()
        .filter(|event| event.remediation.status == RemediationStatus::AppliedAndRerunPassed)
        .count();
    let unresolved = record.unresolved_findings();
    AcceptanceTaskBox {
        title: copy(locale, "Sego 验收状态", "Sego acceptance status").to_string(),
        state_label: state_label(record.acceptance_state, locale).to_string(),
        node_review_count,
        task_end_review_count,
        full_review_count,
        automatically_remediated_count,
        unresolved_count: unresolved.len(),
        primary_reason: primary_reason(record, unresolved.first().copied(), locale),
        next_action: next_action_label(record.next_action_code, locale).to_string(),
        details_label: copy(locale, "查看验收记录", "View acceptance record").to_string(),
        evidence_label: copy(locale, "查看完整证据", "View full evidence").to_string(),
    }
}

#[must_use]
pub fn render_task_box(record: &AcceptanceRecord, locale: AcceptanceLocale) -> String {
    let box_model = task_box_for(record, locale);
    let reviews_label = copy(locale, "本任务审查", "Reviews in this task");
    let node_label = copy(locale, "节点 review", "Node reviews");
    let task_end_label = copy(locale, "任务完成 review", "Task-end reviews");
    let full_label = copy(locale, "Full review", "Full reviews");
    let auto_label = copy(locale, "自动处理", "Automatically remediated");
    let unresolved_label = copy(locale, "待处理", "Unresolved");
    let next_label = copy(locale, "下一步", "Next action");
    format!(
        "{title}: {state}\n\n{reviews_label}:\n- {node_label}: {node}\n- {task_end_label}: {task_end}\n- {full_label}: {full}\n- {auto_label}: {automatic}\n- {unresolved_label}: {unresolved}\n\n{reason}\n{next_label}: {next_action}\n[{details}]  [{evidence}]",
        title = box_model.title,
        state = box_model.state_label,
        node = box_model.node_review_count,
        task_end = box_model.task_end_review_count,
        full = box_model.full_review_count,
        automatic = box_model.automatically_remediated_count,
        unresolved = box_model.unresolved_count,
        reason = box_model.primary_reason,
        next_action = box_model.next_action,
        details = box_model.details_label,
        evidence = box_model.evidence_label,
    )
}

#[must_use]
pub fn render_compact_html(record: &AcceptanceRecord, locale: AcceptanceLocale) -> String {
    let box_model = task_box_for(record, locale);
    let unresolved = record.unresolved_findings();
    let finding_block = unresolved.first().map_or_else(
        || {
            format!(
                "<p class=\"muted\">{}</p>",
                escape_html(copy(locale, "没有未解决 finding。", "No unresolved findings."))
            )
        },
        |finding| render_original_finding(finding, locale),
    );
    format!(
        r#"<!doctype html>
<html lang="{lang}">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{title}</title>
  <style>
    body {{ font-family: "Segoe UI", "Microsoft YaHei", sans-serif; margin: 0; background: #f5f7fb; color: #172033; }}
    main {{ max-width: 760px; margin: 28px auto; padding: 0 16px; }}
    section {{ background: #fff; border: 1px solid #d8deea; border-radius: 14px; padding: 22px; margin-bottom: 14px; }}
    h1, h2 {{ margin-top: 0; }} .state {{ font-weight: 700; font-size: 20px; }}
    dl {{ display: grid; grid-template-columns: 1fr 1fr; gap: 10px 18px; }} dt {{ color: #5b6578; }} dd {{ margin: 0; }}
    .evidence {{ background: #f5f7fb; border-radius: 8px; padding: 12px; white-space: pre-wrap; overflow-wrap: anywhere; }}
    .muted {{ color: #5b6578; }} footer {{ color: #5b6578; font-size: 14px; }}
  </style>
</head>
<body>
<main>
  <section>
    <h1>{title}</h1>
    <p class="state">{state}</p>
    <p>{reason}</p>
    <p><strong>{next_label}:</strong> {next_action}</p>
  </section>
  <section>
    <h2>{review_label}</h2>
    <dl>
      <dt>{node_label}</dt><dd>{node}</dd>
      <dt>{task_end_label}</dt><dd>{task_end}</dd>
      <dt>{full_label}</dt><dd>{full}</dd>
      <dt>{automatic_label}</dt><dd>{automatic}</dd>
      <dt>{unresolved_label}</dt><dd>{unresolved}</dd>
    </dl>
  </section>
  <section>
    <h2>{risk_label}</h2>
    {finding_block}
  </section>
  <footer>{boundary}</footer>
</main>
</body>
</html>"#,
        lang = if locale == AcceptanceLocale::ZhCn { "zh-CN" } else { "en" },
        title = escape_html(&box_model.title),
        state = escape_html(&box_model.state_label),
        reason = escape_html(&box_model.primary_reason),
        next_label = escape_html(copy(locale, "下一步", "Next action")),
        next_action = escape_html(&box_model.next_action),
        review_label = escape_html(copy(locale, "本任务审查", "Reviews in this task")),
        node_label = escape_html(copy(locale, "节点 review", "Node reviews")),
        node = box_model.node_review_count,
        task_end_label = escape_html(copy(locale, "任务完成 review", "Task-end reviews")),
        task_end = box_model.task_end_review_count,
        full_label = escape_html(copy(locale, "Full review", "Full reviews")),
        full = box_model.full_review_count,
        automatic_label = escape_html(copy(locale, "自动处理", "Automatically remediated")),
        automatic = box_model.automatically_remediated_count,
        unresolved_label = escape_html(copy(locale, "待处理", "Unresolved")),
        unresolved = box_model.unresolved_count,
        risk_label =
            escape_html(copy(locale, "关键风险（原始证据）", "Key risk (original evidence)")),
        finding_block = finding_block,
        boundary = escape_html(copy(
            locale,
            "Sego 提供验收记录，不是 release approval 或 security certification。",
            "Sego provides an acceptance record, not release approval or security certification.",
        )),
    )
}

fn review_counts(record: &AcceptanceRecord) -> (usize, usize, usize) {
    let mut node = 0;
    let mut task_end = 0;
    let mut full = 0;
    for event in &record.review_events {
        match event.review_kind {
            ReviewKind::Node => node += 1,
            ReviewKind::TaskEnd => task_end += 1,
            ReviewKind::Full => full += 1,
        }
    }
    (node, task_end, full)
}

fn primary_reason(
    record: &AcceptanceRecord,
    primary_finding: Option<&UnresolvedFinding>,
    locale: AcceptanceLocale,
) -> String {
    if let Some(finding) = primary_finding {
        return match locale {
            AcceptanceLocale::ZhCn => format!("关键风险：{}", finding.title_original),
            AcceptanceLocale::En => format!("Key risk: {}", finding.title_original),
        };
    }
    let code = record.reason_codes.first().copied();
    match (locale, code) {
        (AcceptanceLocale::ZhCn, Some(AcceptanceReasonCode::FullReviewCoverageGap)) => {
            "任务完成审查存在证据覆盖缺口。".to_string()
        }
        (AcceptanceLocale::En, Some(AcceptanceReasonCode::FullReviewCoverageGap)) => {
            "The task-end review has an evidence coverage gap.".to_string()
        }
        (AcceptanceLocale::ZhCn, Some(AcceptanceReasonCode::UnreliableArtifact)) => {
            "审查 artifact 不可靠，不能当作干净结论。".to_string()
        }
        (AcceptanceLocale::En, Some(AcceptanceReasonCode::UnreliableArtifact)) => {
            "The review artifact is unreliable and cannot be treated as a clean result.".to_string()
        }
        (AcceptanceLocale::ZhCn, _) => "本任务没有未解决 finding。".to_string(),
        (AcceptanceLocale::En, _) => "This task has no unresolved findings.".to_string(),
    }
}

fn render_original_finding(finding: &UnresolvedFinding, locale: AcceptanceLocale) -> String {
    let evidence_label = copy(locale, "原始证据", "Original evidence");
    let location_label = copy(locale, "位置", "Location");
    let location = finding
        .line
        .map_or_else(|| finding.file.clone(), |line| format!("{}:{line}", finding.file));
    format!(
        "<article><strong>{}</strong><p>{}: {}</p><div class=\"evidence\"><strong>{}</strong>\n{}</div></article>",
        escape_html(&finding.title_original),
        escape_html(location_label),
        escape_html(&location),
        escape_html(evidence_label),
        escape_html(&finding.evidence_original),
    )
}

const fn copy(locale: AcceptanceLocale, zh_cn: &'static str, en: &'static str) -> &'static str {
    match locale {
        AcceptanceLocale::ZhCn => zh_cn,
        AcceptanceLocale::En => en,
    }
}

const fn state_label(state: AcceptanceState, locale: AcceptanceLocale) -> &'static str {
    match (locale, state) {
        (AcceptanceLocale::ZhCn, AcceptanceState::NeedsAttention) => "需要处理",
        (AcceptanceLocale::ZhCn, AcceptanceState::NeedsReview) => "请复核",
        (AcceptanceLocale::ZhCn, AcceptanceState::ReadyForNormalVerification) => "可进入常规验证",
        (AcceptanceLocale::En, AcceptanceState::NeedsAttention) => "Needs attention",
        (AcceptanceLocale::En, AcceptanceState::NeedsReview) => "Needs review",
        (AcceptanceLocale::En, AcceptanceState::ReadyForNormalVerification) => {
            "Ready for normal verification"
        }
    }
}

const fn next_action_label(action: NextActionCode, locale: AcceptanceLocale) -> &'static str {
    match (locale, action) {
        (AcceptanceLocale::ZhCn, NextActionCode::ResolveBlockerBeforeContinuing) => {
            "先处理阻塞项，再继续任务。"
        }
        (AcceptanceLocale::ZhCn, NextActionCode::ConfirmOrFixRiskThenRerun) => {
            "确认或修复风险后重新运行 Sego。"
        }
        (AcceptanceLocale::ZhCn, NextActionCode::ReviewCoverageGapBeforeRelease) => {
            "在发布前复核覆盖缺口或记录接受风险。"
        }
        (AcceptanceLocale::ZhCn, NextActionCode::ContinueNormalVerification) => {
            "继续常规测试和人工判断。"
        }
        (AcceptanceLocale::En, NextActionCode::ResolveBlockerBeforeContinuing) => {
            "Resolve the blocker before continuing the task."
        }
        (AcceptanceLocale::En, NextActionCode::ConfirmOrFixRiskThenRerun) => {
            "Confirm or fix the risk, then rerun Sego."
        }
        (AcceptanceLocale::En, NextActionCode::ReviewCoverageGapBeforeRelease) => {
            "Review the coverage gap or record accepted risk before release."
        }
        (AcceptanceLocale::En, NextActionCode::ContinueNormalVerification) => {
            "Continue normal tests and human judgment."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{render_compact_html, render_task_box, task_box_for, AcceptanceLocale};
    use runtime::AcceptanceRecord;

    const MIXED_RECORD_FIXTURE: &str =
        include_str!("../tests/fixtures/task-acceptance-record-mixed.json");

    fn fixture_record() -> AcceptanceRecord {
        serde_json::from_str(MIXED_RECORD_FIXTURE)
            .expect("fixture should match acceptance contract")
    }

    #[test]
    fn mixed_task_fixture_matches_the_deterministic_record_aggregation() {
        let fixture = fixture_record();
        let recomputed = AcceptanceRecord::from_review_events(
            fixture.task_id.clone(),
            fixture.review_events.clone(),
        );

        assert_eq!(fixture.acceptance_state, recomputed.acceptance_state);
        assert_eq!(fixture.reason_codes, recomputed.reason_codes);
        assert_eq!(fixture.next_action_code, recomputed.next_action_code);
        assert_eq!(fixture.evidence_links, recomputed.evidence_links);
    }

    #[test]
    fn localized_task_boxes_keep_task_facts_and_original_finding_text() {
        let record = fixture_record();
        let zh = task_box_for(&record, AcceptanceLocale::ZhCn);
        let en = task_box_for(&record, AcceptanceLocale::En);

        assert_eq!(zh.state_label, "请复核");
        assert_eq!(en.state_label, "Needs review");
        assert_eq!(zh.node_review_count, en.node_review_count);
        assert_eq!(zh.full_review_count, en.full_review_count);
        assert_eq!(zh.unresolved_count, en.unresolved_count);
        assert!(zh.primary_reason.contains("Missing authorization check"));
        assert!(en.primary_reason.contains("Missing authorization check"));
    }

    #[test]
    fn task_box_renders_native_summary_copy_without_changing_evidence() {
        let rendered = render_task_box(&fixture_record(), AcceptanceLocale::ZhCn);
        assert!(rendered.contains("Sego 验收状态: 请复核"));
        assert!(rendered.contains("节点 review: 1"));
        assert!(rendered.contains("Full review: 1"));
        assert!(rendered.contains("Missing authorization check before account update"));
    }

    #[test]
    fn compact_html_uses_localized_chrome_and_escaped_original_evidence() {
        let mut record = fixture_record();
        record.review_events[1].unresolved_findings[0].evidence_original =
            "<unsafe>&evidence".to_string();
        let html = render_compact_html(&record, AcceptanceLocale::En);
        assert!(html.contains("Sego acceptance status"));
        assert!(html.contains("Key risk (original evidence)"));
        assert!(html.contains("&lt;unsafe&gt;&amp;evidence"));
        assert!(html.contains("not release approval or security certification"));
    }
}
