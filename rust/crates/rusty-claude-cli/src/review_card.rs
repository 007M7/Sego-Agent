use std::path::Path;

use runtime::{ReviewFinding, ReviewParseStatus, ReviewSeverity};

pub const CARD_TITLE: &str = "Sego 验收卡";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewCardConfidence {
    Green,
    Yellow,
    Red,
}

impl ReviewCardConfidence {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Green => "Green",
            Self::Yellow => "Yellow",
            Self::Red => "Red",
        }
    }

    #[must_use]
    const fn css_class(self) -> &'static str {
        match self {
            Self::Green => "green",
            Self::Yellow => "yellow",
            Self::Red => "red",
        }
    }
}

pub struct ReviewCardData<'a> {
    pub id: &'a str,
    pub scope: &'a str,
    pub finding_count: usize,
    pub highest_severity: Option<ReviewSeverity>,
    pub parse_status: ReviewParseStatus,
    pub findings: &'a [ReviewFinding],
    pub json_path: &'a Path,
    pub markdown_path: &'a Path,
}

#[must_use]
pub fn confidence_for(
    parse_status: ReviewParseStatus,
    findings: &[ReviewFinding],
) -> ReviewCardConfidence {
    if parse_status != ReviewParseStatus::Structured
        || findings.iter().any(|finding| finding.severity == ReviewSeverity::Critical)
    {
        ReviewCardConfidence::Red
    } else if findings
        .iter()
        .any(|finding| matches!(finding.severity, ReviewSeverity::High | ReviewSeverity::Medium))
    {
        ReviewCardConfidence::Yellow
    } else {
        ReviewCardConfidence::Green
    }
}

#[must_use]
pub fn top_risks(findings: &[ReviewFinding]) -> Vec<&ReviewFinding> {
    let mut indexed = findings.iter().enumerate().collect::<Vec<_>>();
    indexed.sort_by_key(|(index, finding)| (finding.severity, *index));
    indexed.into_iter().take(3).map(|(_, finding)| finding).collect()
}

#[must_use]
pub fn next_action(confidence: ReviewCardConfidence) -> &'static str {
    match confidence {
        ReviewCardConfidence::Red => "在提交或合并前修复或明确处理最高严重性风险。",
        ReviewCardConfidence::Yellow => "审阅首要风险；如确认有效请修复，并在发布前重新运行 Sego。",
        ReviewCardConfidence::Green => "如有需要请查看完整报告，并继续执行常规测试和人工判断。",
    }
}

#[must_use]
pub fn ready_gate(confidence: ReviewCardConfidence) -> &'static str {
    match confidence {
        ReviewCardConfidence::Red => "阻塞：先处理 critical finding 或不可靠 artifact。",
        ReviewCardConfidence::Yellow => "待人工复核：处理或确认首要风险后重新运行 Sego。",
        ReviewCardConfidence::Green => "可继续常规验证：不等于 release approval。",
    }
}

#[must_use]
pub fn render_terminal_summary(data: &ReviewCardData<'_>, card_path: &Path) -> String {
    let confidence = confidence_for(data.parse_status, data.findings);
    let highest = data.highest_severity.map_or("none", ReviewSeverity::label);
    let top_risks = top_risks(data.findings);
    let mut lines = vec![
        CARD_TITLE.to_string(),
        format!("  Review: {}", data.id),
        format!("  Merge confidence: {}", confidence.label()),
        format!("  Findings: {} | Highest: {highest}", data.finding_count),
        "  Top risks:".to_string(),
    ];

    if top_risks.is_empty() {
        lines.push("    - 未发现结构化风险；仍请结合完整报告和常规测试判断。".to_string());
    } else {
        for risk in top_risks {
            lines.push(format!("    - [{}] {}", risk.severity.label(), risk.title));
        }
    }

    lines.extend([
        format!("  Next action: {}", next_action(confidence)),
        format!("  Card: {}", card_path.display()),
    ]);
    lines.join("\n")
}

#[must_use]
pub fn render_html(data: &ReviewCardData<'_>) -> String {
    let confidence = confidence_for(data.parse_status, data.findings);
    let highest = data.highest_severity.map_or("none", ReviewSeverity::label);
    let risks = top_risks(data.findings);
    let risk_items = if risks.is_empty() {
        String::from("<li class=\"empty\">未发现结构化风险；仍请查看完整报告并执行常规测试。</li>")
    } else {
        risks.into_iter().map(render_risk).collect::<Vec<_>>().join("\n")
    };
    let json_link = artifact_link("JSON artifact", data.json_path);
    let markdown_link = artifact_link("Markdown report", data.markdown_path);

    format!(
        r#"<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{title} · {id}</title>
  <style>
    :root {{ color-scheme: light; font-family: "Segoe UI", "Microsoft YaHei", sans-serif; background: #f5f7fb; color: #172033; }}
    body {{ margin: 0; padding: 32px 16px; }}
    main {{ max-width: 820px; margin: 0 auto; background: #ffffff; border: 1px solid #d8deea; border-radius: 16px; box-shadow: 0 12px 36px rgba(23, 32, 51, .08); overflow: hidden; }}
    header {{ padding: 28px; background: #172033; color: #ffffff; }}
    h1 {{ margin: 0 0 8px; font-size: 28px; }}
    .meta {{ margin: 0; color: #d7def0; overflow-wrap: anywhere; }}
    section {{ padding: 24px 28px; border-top: 1px solid #e6eaf2; }}
    h2 {{ margin: 0 0 14px; font-size: 18px; }}
    .status {{ display: inline-block; padding: 7px 12px; border-radius: 999px; font-weight: 700; }}
    .green {{ background: #d9f7e5; color: #0b6b38; }} .yellow {{ background: #fff3cc; color: #8a5700; }} .red {{ background: #ffe1e1; color: #a21818; }}
    dl {{ display: grid; grid-template-columns: minmax(130px, 1fr) 2fr; gap: 10px 18px; margin: 18px 0 0; }}
    dt {{ color: #5b6578; }} dd {{ margin: 0; overflow-wrap: anywhere; }}
    ol {{ margin: 0; padding-left: 22px; }} li {{ margin: 12px 0; }} .risk-meta {{ color: #5b6578; font-size: 14px; margin: 4px 0; }}
    .evidence {{ background: #f5f7fb; border-radius: 8px; padding: 10px; white-space: pre-wrap; overflow-wrap: anywhere; }}
    .boundary {{ background: #eef2f9; color: #3e4a60; }} .links a {{ margin-right: 16px; }} .empty {{ color: #5b6578; }}
  </style>
</head>
<body>
  <main>
    <header>
      <h1>{title}</h1>
      <p class="meta">Review ID: {id}</p>
    </header>
    <section aria-labelledby="decision-heading">
      <h2 id="decision-heading">验收决策摘要</h2>
      <p><span class="status {confidence_class}">Merge confidence: {confidence}</span></p>
      <dl>
        <dt>Scope</dt><dd>{scope}</dd>
        <dt>Highest severity</dt><dd>{highest}</dd>
        <dt>Finding count</dt><dd>{finding_count}</dd>
        <dt>Parse status</dt><dd>{parse_status}</dd>
        <dt>Ready gate</dt><dd>{ready_gate}</dd>
        <dt>Next action</dt><dd>{next_action}</dd>
      </dl>
    </section>
    <section aria-labelledby="risks-heading">
      <h2 id="risks-heading">Top 3 risks</h2>
      <ol>{risk_items}</ol>
    </section>
    <section class="links" aria-labelledby="artifacts-heading">
      <h2 id="artifacts-heading">Artifacts</h2>
      {json_link}
      {markdown_link}
    </section>
    <section class="boundary" aria-label="产品边界">
      Sego 生成的是审阅证明，不是 release approval 或 security certification。它不替代人工审阅、测试、CI、静态分析或合规流程。
    </section>
  </main>
</body>
</html>"#,
        title = escape_html(CARD_TITLE),
        id = escape_html(data.id),
        confidence = confidence.label(),
        confidence_class = confidence.css_class(),
        scope = escape_html(data.scope),
        highest = escape_html(highest),
        finding_count = data.finding_count,
        parse_status = escape_html(data.parse_status.label()),
        ready_gate = escape_html(ready_gate(confidence)),
        next_action = escape_html(next_action(confidence)),
        risk_items = risk_items,
        json_link = json_link,
        markdown_link = markdown_link,
    )
}

fn render_risk(finding: &ReviewFinding) -> String {
    let location = finding
        .line
        .map_or_else(|| finding.file.clone(), |line| format!("{}:{line}", finding.file));
    let evidence_status = finding.evidence_status.map_or("not_recorded", |status| status.label());
    format!(
        "<li><strong>[{}] {}</strong><p class=\"risk-meta\">{} · evidence: {}</p><div class=\"evidence\">{}</div></li>",
        escape_html(finding.severity.label()),
        escape_html(&finding.title),
        escape_html(&location),
        escape_html(evidence_status),
        escape_html(&finding.evidence),
    )
}

fn artifact_link(label: &str, path: &Path) -> String {
    if path.is_file() {
        format!("<a href=\"{}\">{}</a>", escape_html(&local_file_url(path)), escape_html(label))
    } else {
        format!("<span>{}: unavailable</span>", escape_html(label))
    }
}

#[must_use]
pub fn local_file_url(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    let prefix = if raw.starts_with('/') { "file://" } else { "file:///" };
    format!("{prefix}{}", percent_encode_file_path(&raw))
}

fn percent_encode_file_path(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/' | b':') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[must_use]
pub fn escape_html(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::{
        confidence_for, escape_html, local_file_url, render_html, top_risks, ReviewCardConfidence,
        ReviewCardData,
    };
    use runtime::{ReviewFinding, ReviewParseStatus, ReviewSeverity};
    use std::path::Path;

    fn finding(severity: ReviewSeverity, title: &str) -> ReviewFinding {
        ReviewFinding {
            id: "finding-test".to_string(),
            severity,
            file: "src/lib.rs".to_string(),
            line: Some(12),
            title: title.to_string(),
            evidence: "evidence".to_string(),
            risk: "risk".to_string(),
            suggestion: "suggestion".to_string(),
            confidence: 0.8,
            verification_hint: None,
            evidence_status: None,
        }
    }

    #[test]
    fn confidence_mapping_follows_product_rules() {
        assert_eq!(confidence_for(ReviewParseStatus::Structured, &[]), ReviewCardConfidence::Green);
        assert_eq!(
            confidence_for(
                ReviewParseStatus::Structured,
                &[finding(ReviewSeverity::Medium, "medium")]
            ),
            ReviewCardConfidence::Yellow
        );
        assert_eq!(
            confidence_for(
                ReviewParseStatus::Structured,
                &[finding(ReviewSeverity::Critical, "critical")]
            ),
            ReviewCardConfidence::Red
        );
        assert_eq!(
            confidence_for(
                ReviewParseStatus::FallbackRawText,
                &[finding(ReviewSeverity::Low, "low")]
            ),
            ReviewCardConfidence::Red
        );
    }

    #[test]
    fn top_risks_orders_by_severity_and_keeps_only_three() {
        let findings = vec![
            finding(ReviewSeverity::Info, "info"),
            finding(ReviewSeverity::High, "high"),
            finding(ReviewSeverity::Critical, "critical"),
            finding(ReviewSeverity::Medium, "medium"),
        ];
        let titles =
            top_risks(&findings).into_iter().map(|item| item.title.as_str()).collect::<Vec<_>>();
        assert_eq!(titles, vec!["critical", "high", "medium"]);
    }

    #[test]
    fn html_escapes_finding_title_file_and_evidence() {
        let mut unsafe_finding = finding(ReviewSeverity::High, "<script>alert('title')</script>");
        unsafe_finding.file = "src/<unsafe>.rs".to_string();
        unsafe_finding.evidence = "<&\"'>".to_string();
        let data = ReviewCardData {
            id: "review-unsafe",
            scope: "workspace",
            finding_count: 1,
            highest_severity: Some(ReviewSeverity::High),
            parse_status: ReviewParseStatus::Structured,
            findings: &[unsafe_finding],
            json_path: Path::new("missing.json"),
            markdown_path: Path::new("missing.md"),
        };
        let html = render_html(&data);
        assert!(!html.contains("<script>alert('title')</script>"));
        assert!(html.contains("&lt;script&gt;alert(&#39;title&#39;)&lt;/script&gt;"));
        assert!(html.contains("src/&lt;unsafe&gt;.rs"));
        assert!(html.contains("&lt;&amp;&quot;&#39;&gt;"));
        assert_eq!(escape_html("<&\"'>"), "&lt;&amp;&quot;&#39;&gt;");
    }

    #[test]
    fn html_structure_is_stable_for_golden_review() {
        let findings = vec![finding(ReviewSeverity::Low, "Document decision boundary")];
        let data = ReviewCardData {
            id: "review-golden",
            scope: "staged",
            finding_count: 1,
            highest_severity: Some(ReviewSeverity::Low),
            parse_status: ReviewParseStatus::Structured,
            findings: &findings,
            json_path: Path::new("missing.json"),
            markdown_path: Path::new("missing.md"),
        };
        let html = render_html(&data);
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("<h1>Sego 验收卡</h1>"));
        assert!(html.contains("Merge confidence: Green"));
        assert!(html.contains("<h2 id=\"risks-heading\">Top 3 risks</h2>"));
        assert!(html
            .contains("Sego 生成的是审阅证明，不是 release approval 或 security certification。"));
    }

    #[test]
    fn local_file_urls_support_windows_paths_with_spaces() {
        assert_eq!(
            local_file_url(Path::new(r"C:\Sego max\.sego\reviews\latest card.html")),
            "file:///C:/Sego%20max/.sego/reviews/latest%20card.html"
        );
    }
}
