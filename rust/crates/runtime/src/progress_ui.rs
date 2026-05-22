// ============================================================================
// Sego Agent — Progress UI (Rust port of src/progress_ui.py)
// Compact phase-based progress bars + spinner.
// Only minimal status lines are printed to stdout.
// All detailed execution info is routed to the work log.
// ============================================================================

use std::collections::HashMap;
use std::fmt;
use std::io::{self, Write};
use std::time::Instant;

// ---------------------------------------------------------------------------
// ANSI terminal helpers (zero-dep, mirrors Python _Style)
// ---------------------------------------------------------------------------
mod ansi {
    pub const RESET: &str = "\x1b[0m";
    pub const BOLD: &str = "\x1b[1m";
    pub const DIM: &str = "\x1b[2m";
    pub const GREEN: &str = "\x1b[32m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const CYAN: &str = "\x1b[36m";
    pub const RED: &str = "\x1b[31m";
    pub const WHITE: &str = "\x1b[37m";
}

use ansi::{CYAN, RESET, GREEN, RED, DIM, BOLD, YELLOW, WHITE};

// ---------------------------------------------------------------------------
// Spinner frames
// ---------------------------------------------------------------------------
const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

// ---------------------------------------------------------------------------
// Phase status
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

impl fmt::Display for PhaseStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Skipped => write!(f, "skipped"),
        }
    }
}

impl PhaseStatus {
    fn ansi_icon(&self) -> String {
        match self {
            Self::Pending => " ".to_string(),
            Self::Running => format!("{CYAN}◷{RESET}"),
            Self::Completed => format!("{GREEN}✔{RESET}"),
            Self::Failed => format!("{RED}✘{RESET}"),
            Self::Skipped => format!("{DIM}○{RESET}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Phase — a single named phase in the execution pipeline
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct Phase {
    pub name: String,
    pub status: PhaseStatus,
    pub message: String,
    pub started_at: Option<Instant>,
    pub finished_at: Option<Instant>,
    pub elapsed_ms: f64,
    /// Routed to work log, not displayed inline.
    pub detail: String,
    /// Routed to work log.
    pub error: String,
}

impl Phase {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: PhaseStatus::Pending,
            message: String::new(),
            started_at: None,
            finished_at: None,
            elapsed_ms: 0.0,
            detail: String::new(),
            error: String::new(),
        }
    }

    pub fn start(&mut self) {
        self.status = PhaseStatus::Running;
        self.started_at = Some(Instant::now());
    }

    pub fn complete(&mut self, message: impl Into<String>, detail: impl Into<String>) {
        self.status = PhaseStatus::Completed;
        self.finished_at = Some(Instant::now());
        if let Some(start) = self.started_at {
            self.elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        }
        self.message = message.into();
        self.detail = detail.into();
    }

    pub fn fail(&mut self, message: impl Into<String>, error: impl Into<String>) {
        self.status = PhaseStatus::Failed;
        self.finished_at = Some(Instant::now());
        if let Some(start) = self.started_at {
            self.elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        }
        self.message = message.into();
        self.error = error.into();
    }

    pub fn skip(&mut self) {
        self.status = PhaseStatus::Skipped;
    }
}

// ---------------------------------------------------------------------------
// Progress UI engine
// ---------------------------------------------------------------------------
pub struct ProgressUI<W: Write> {
    title: String,
    phases: Vec<Phase>,
    phase_index: HashMap<String, usize>,
    spinner_idx: usize,
    started: bool,
    finished: bool,
    compact: bool,
    out: W,
}

impl<W: Write> ProgressUI<W> {
    #[must_use]
    pub fn new(title: impl Into<String>, out: W) -> Self {
        Self {
            title: title.into(),
            phases: Vec::new(),
            phase_index: HashMap::new(),
            spinner_idx: 0,
            started: false,
            finished: false,
            compact: true,
            out,
        }
    }

    /// Add a phase and return its index.
    pub fn add_phase(&mut self, key: impl Into<String>, name: impl Into<String>) -> usize {
        let key = key.into();
        let idx = self.phases.len();
        self.phase_index.insert(key, idx);
        self.phases.push(Phase::new(name));
        idx
    }

    /// Look up a mutable phase by its key.
    pub fn phase(&mut self, key: &str) -> &mut Phase {
        let idx = self.phase_index[key];
        &mut self.phases[idx]
    }

    /// Look up a mutable phase by its numeric index.
    pub fn phase_idx(&mut self, idx: usize) -> &mut Phase {
        &mut self.phases[idx]
    }

    /// Look up a phase key by index (fallback to "...").  
    #[must_use]
    pub fn phase_name(&self, idx: usize) -> &str {
        self.phases.get(idx).map_or("...", |p| p.name.as_str())
    }

    /// Look up a phase message by index.
    #[must_use]
    pub fn phase_message(&self, idx: usize) -> &str {
        self.phases.get(idx).map_or("", |p| p.message.as_str())
    }

    // -- lifecycle ----------------------------------------------------------

    pub fn start(&mut self) -> io::Result<()> {
        self.started = true;
        self.print_header()
    }

    pub fn finish(&mut self, final_message: &str) -> io::Result<()> {
        self.finished = true;
        if !self.compact {
            self.render_full()?;
        }
        self.print_footer(final_message)
    }

    pub fn set_compact(&mut self, value: bool) {
        self.compact = value;
    }

    /// Render a single compact progress bar line (overwrites previous line
    /// with `\r`).
    pub fn render_compact(&mut self, current_phase_idx: Option<usize>) -> io::Result<()> {
        let completed = self.phases.iter().filter(|p| p.status == PhaseStatus::Completed).count();
        let failed = self.phases.iter().filter(|p| p.status == PhaseStatus::Failed).count();
        let total = self.phases.len();

        let spinner = SPINNER_FRAMES[self.spinner_idx % SPINNER_FRAMES.len()];
        self.spinner_idx += 1;

        let bar = draw_bar(completed + failed, total, 20);

        let running_name = current_phase_idx.map_or("...", |i| self.phase_name(i));

        let msg = current_phase_idx
            .and_then(|i| {
                let m = self.phase_message(i);
                if m.is_empty() {
                    None
                } else {
                    Some(m)
                }
            })
            .map(|m| format!(" — {DIM}{m}{RESET}"))
            .unwrap_or_default();

        let spinner_color = if failed > 0 { RED } else { CYAN };

        let line =
            format!(" {spinner_color}{spinner}{RESET} {bar} {BOLD}{running_name}{RESET}{msg}");

        write!(self.out, "\r{line}\x1b[K")?;
        self.out.flush()
    }

    // -- extract work-log data ----------------------------------------------

    #[must_use]
    pub fn extract_log_data(&self) -> Vec<PhaseLogEntry> {
        self.phases
            .iter()
            .map(|p| PhaseLogEntry {
                phase: p.name.clone(),
                status: p.status.to_string(),
                elapsed_ms: p.elapsed_ms,
                message: p.message.clone(),
                detail: p.detail.clone(),
                error: p.error.clone(),
            })
            .collect()
    }

    /// Consume self and return the inner writer.
    pub fn into_inner(self) -> W {
        self.out
    }

    // -- internal render helpers --------------------------------------------

    fn print_header(&mut self) -> io::Result<()> {
        let width = (self.title.len() + 4).min(78);
        let bar: String = "━".repeat(width);
        writeln!(self.out, "\n{CYAN}{BOLD}{bar}{RESET}")?;
        writeln!(self.out, "{BOLD}  {}{RESET}", self.title)?;
        writeln!(self.out, "{CYAN}{bar}{RESET}\n")
    }

    fn print_footer(&mut self, final_message: &str) -> io::Result<()> {
        let completed = self.phases.iter().filter(|p| p.status == PhaseStatus::Completed).count();
        let failed = self.phases.iter().filter(|p| p.status == PhaseStatus::Failed).count();
        let total = self.phases.len();
        let total_ms: f64 = self.phases.iter().map(|p| p.elapsed_ms).sum();

        let bar: String = "━".repeat(34);
        writeln!(self.out, "\n{CYAN}{bar}{RESET}")?;

        let color = if failed > 0 { RED } else { GREEN };
        writeln!(
            self.out,
            "  {color}{completed}/{total} phases completed ({}){RESET}",
            elapsed_str(total_ms)
        )?;

        if failed > 0 {
            let names: Vec<&str> = self
                .phases
                .iter()
                .filter(|p| p.status == PhaseStatus::Failed)
                .map(|p| p.name.as_str())
                .collect();
            writeln!(self.out, "  {RED}Failed: {}{RESET}", names.join(", "))?;
        }

        if !final_message.is_empty() {
            writeln!(self.out, "  {final_message}")?;
        }

        writeln!(self.out, "{CYAN}{bar}{RESET}\n")
    }

    fn render_full(&mut self) -> io::Result<()> {
        for p in &self.phases {
            let icon = p.status.ansi_icon();
            let msg = if p.message.is_empty() { String::new() } else { format!(" {}", p.message) };
            let elapsed = if p.elapsed_ms > 0.0 {
                format!(" {}", elapsed_str(p.elapsed_ms))
            } else {
                String::new()
            };
            writeln!(self.out, "  {icon} {}{msg}{elapsed}", p.name)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Bar drawing
// ---------------------------------------------------------------------------
fn draw_bar(completed: usize, total: usize, width: usize) -> String {
    let ratio = (completed as f64 / total.max(1) as f64).min(1.0);
    let filled = (ratio * width as f64) as usize;
    let pct = (ratio * 100.0) as usize;

    let (bar_color, pct_color) = if ratio >= 1.0 {
        (GREEN, GREEN)
    } else if ratio >= 0.5 {
        (YELLOW, YELLOW)
    } else {
        (CYAN, WHITE)
    };

    let blocks: String = "█".repeat(filled);
    let spaces: String = "░".repeat(width - filled);

    format!("{bar_color}[{bar_color}{blocks}{DIM}{spaces}{bar_color}] {pct_color}{pct:>3}%{RESET}")
}

// ---------------------------------------------------------------------------
// Elapsed formatting
// ---------------------------------------------------------------------------
fn elapsed_str(ms: f64) -> String {
    if ms < 1000.0 {
        format!("{ms:5.0}ms")
    } else {
        format!("{:5.1}s", ms / 1000.0)
    }
}

// ---------------------------------------------------------------------------
// Serializable work-log entry
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PhaseLogEntry {
    pub phase: String,
    pub status: String,
    pub elapsed_ms: f64,
    pub message: String,
    pub detail: String,
    pub error: String,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    // -- Phase --------------------------------------------------------------

    #[test]
    fn phase_defaults() {
        let p = Phase::new("test");
        assert_eq!(p.status, PhaseStatus::Pending);
        assert!(p.message.is_empty());
        assert_eq!(p.elapsed_ms, 0.0);
        assert!(p.detail.is_empty());
        assert!(p.error.is_empty());
    }

    #[test]
    fn phase_complete_sets_elapsed() {
        let mut p = Phase::new("build");
        p.start();
        std::thread::sleep(std::time::Duration::from_millis(5));
        p.complete("done", "all files compiled");
        assert_eq!(p.status, PhaseStatus::Completed);
        assert!(p.elapsed_ms > 0.0);
        assert_eq!(p.message, "done");
        assert_eq!(p.detail, "all files compiled");
    }

    #[test]
    fn phase_fail_sets_error() {
        let mut p = Phase::new("connect");
        p.start();
        p.fail("timeout", "connection refused");
        assert_eq!(p.status, PhaseStatus::Failed);
        assert_eq!(p.error, "connection refused");
    }

    #[test]
    fn phase_skip() {
        let mut p = Phase::new("optional");
        p.skip();
        assert_eq!(p.status, PhaseStatus::Skipped);
    }

    // -- Bar drawing --------------------------------------------------------

    #[test]
    fn bar_zero_percent() {
        let b = draw_bar(0, 6, 20);
        assert!(b.contains("0%") || b.contains("  0%"));
    }

    #[test]
    fn bar_fifty_percent() {
        let b = draw_bar(3, 6, 20);
        assert!(b.contains("50%"));
    }

    #[test]
    fn bar_hundred_percent() {
        let b = draw_bar(6, 6, 20);
        assert!(b.contains("100%"));
    }

    #[test]
    fn bar_zero_total_clamps() {
        let b = draw_bar(0, 0, 20);
        // Should not panic
        assert!(!b.is_empty());
    }

    // -- Elapsed formatting -------------------------------------------------

    #[test]
    fn elapsed_ms() {
        let s = elapsed_str(500.0);
        assert!(s.contains("500ms"));
    }

    #[test]
    fn elapsed_seconds() {
        let s = elapsed_str(1500.0);
        assert!(s.contains("1.5s"));
    }

    // -- ProgressUI lifecycle -----------------------------------------------

    #[test]
    fn add_phase_returns_index() {
        let mut ui = ProgressUI::new("Test", Vec::new());
        let idx = ui.add_phase("a", "Phase A");
        assert_eq!(idx, 0);
        assert_eq!(ui.phases.len(), 1);
        assert_eq!(ui.phases[0].name, "Phase A");
    }

    #[test]
    fn phase_lookup_by_key() {
        let mut ui = ProgressUI::new("Test", Vec::new());
        ui.add_phase("ctx", "Context");
        ui.add_phase("cmd", "Commands");
        assert_eq!(ui.phase("ctx").name, "Context");
        assert_eq!(ui.phase("cmd").name, "Commands");
    }

    #[test]
    fn phase_lookup_by_idx() {
        let mut ui = ProgressUI::new("Test", Vec::new());
        ui.add_phase("a", "Alpha");
        ui.add_phase("b", "Beta");
        assert_eq!(ui.phase_idx(0).name, "Alpha");
        assert_eq!(ui.phase_idx(1).name, "Beta");
    }

    #[test]
    fn finish_sets_finished_flag() {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut ui = ProgressUI::new("Test", &mut buf);
            ui.add_phase("a", "A");
            ui.phase("a").start();
            ui.phase("a").complete("ok", "details");
            ui.finish("All done!").unwrap();
            assert!(ui.finished);
        }
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("1/1 phases completed"));
        assert!(output.contains("All done!"));
    }

    #[test]
    fn finish_shows_failed_count() {
        let mut buf: Vec<u8> = Vec::new();
        {
            let mut ui = ProgressUI::new("Test", &mut buf);
            ui.add_phase("a", "A");
            ui.add_phase("b", "B");
            ui.phase("a").complete("ok", "");
            ui.phase("b").fail("fail", "boom");
            ui.finish("").unwrap();
        }
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("1/2"));
        assert!(output.contains("Failed"));
        assert!(output.contains("B"));
    }

    // -- Log data extraction ------------------------------------------------

    #[test]
    fn extract_log_data_serializes() {
        let mut ui = ProgressUI::new("Test", Vec::new());
        ui.add_phase("a", "Alpha");
        ui.add_phase("b", "Beta");
        ui.phase("a").complete("ok", "details");
        ui.phase("b").fail("nope", "error msg");

        let data = ui.extract_log_data();
        assert_eq!(data.len(), 2);
        assert_eq!(data[0].phase, "Alpha");
        assert_eq!(data[0].status, "completed");
        assert_eq!(data[1].phase, "Beta");
        assert_eq!(data[1].status, "failed");
        assert_eq!(data[1].error, "error msg");

        // Verify it round-trips through serde
        let json = serde_json::to_string(&data).unwrap();
        let parsed: Vec<PhaseLogEntry> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.len(), 2);
    }

    // -- Compact toggle -----------------------------------------------------

    #[test]
    fn compact_on_by_default() {
        let ui = ProgressUI::new("Test", Vec::new());
        assert!(ui.compact);
    }

    #[test]
    fn set_compact_toggles() {
        let mut ui = ProgressUI::new("Test", Vec::new());
        ui.set_compact(false);
        assert!(!ui.compact);
        ui.set_compact(true);
        assert!(ui.compact);
    }

    // -- into_inner ---------------------------------------------------------

    #[test]
    fn into_inner_returns_writer() {
        let ui = ProgressUI::new("Test", Vec::new());
        let buf: Vec<u8> = ui.into_inner();
        assert!(buf.is_empty());
    }
}
