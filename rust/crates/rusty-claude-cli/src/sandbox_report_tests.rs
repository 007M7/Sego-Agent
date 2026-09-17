//! Sandbox report tests (moved out of `main.rs`).

use super::{format_sandbox_report, HookAbortMonitor};
use runtime::HookAbortSignal;
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn sandbox_report_renders_expected_fields() {
    let report = format_sandbox_report(&runtime::SandboxStatus::default());
    assert!(report.contains("Sandbox"));
    assert!(report.contains("Enabled"));
    assert!(report.contains("Filesystem mode"));
    assert!(report.contains("Fallback reason"));
}

#[test]
fn hook_abort_monitor_stops_without_aborting() {
    let abort_signal = HookAbortSignal::new();
    let (ready_tx, ready_rx) = mpsc::channel();
    let monitor = HookAbortMonitor::spawn_with_waiter(
        abort_signal.clone(),
        runtime::TurnAbortSignal::new(),
        false,
        move |stop_rx, abort_signal, turn_abort_signal, _emit_output| {
            ready_tx.send(()).expect("ready signal");
            let _ = stop_rx.recv();
            assert!(!abort_signal.is_aborted());
            assert!(!turn_abort_signal.is_aborted());
        },
    );

    ready_rx.recv().expect("waiter should be ready");
    monitor.stop();

    assert!(!abort_signal.is_aborted());
}

#[test]
fn hook_abort_monitor_propagates_interrupt() {
    let abort_signal = HookAbortSignal::new();
    let (done_tx, done_rx) = mpsc::channel();
    let monitor = HookAbortMonitor::spawn_with_waiter(
        abort_signal.clone(),
        runtime::TurnAbortSignal::new(),
        false,
        move |_stop_rx, abort_signal, turn_abort_signal, _emit_output| {
            abort_signal.abort();
            turn_abort_signal.abort();
            done_tx.send(()).expect("done signal");
        },
    );

    done_rx.recv_timeout(Duration::from_secs(1)).expect("interrupt should complete");
    monitor.stop();

    assert!(abort_signal.is_aborted());
}
