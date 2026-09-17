//! Process-tree reclamation for spawned children.
//!
//! `Child::kill` reaches only the direct child. A shell, build tool or MCP
//! server that spawned its own children leaves them running after a timeout or
//! a cancel - orphaned processes that keep handles, ports and credentials
//! alive (DEV-SEC-16). This module kills the whole tree instead.
//!
//! Platform approach:
//! - Windows: `taskkill /F /T /PID` walks and terminates the tree. No
//!   spawn-time setup is required.
//! - Unix: the caller spawns the child into its own process group
//!   (`CommandExt::process_group(0)`, see [`prepare_process_group`]) so the
//!   whole group can be signalled with `kill -9 -<pid>`.
//!
//! Both paths report what happened rather than only that "the command ran":
//! a target that had already exited is not a failure, but a signal that was
//! refused is. The earlier version discarded the exit status, so a refused kill
//! and a successful one were indistinguishable and no caller could tell an
//! operator that reclamation had failed.

use std::process::Command;

/// What a reclamation attempt achieved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reclamation {
    /// The tree was signalled and the target is gone.
    Reclaimed,
    /// The target had already exited, so nothing was left to reclaim.
    AlreadyGone,
    /// There was no pid to act on (for example a child that never reported an
    /// id), so no attempt was made.
    Skipped,
    /// Reclamation could not be confirmed. Carries the reason for the operator.
    Failed(String),
}

impl Reclamation {
    /// A line an operator should see, or `None` when nothing went wrong.
    ///
    /// `AlreadyGone` and `Skipped` are deliberately quiet: a child that exited
    /// on its own is the normal case, and reporting it would train readers to
    /// ignore reclamation messages.
    #[must_use]
    pub fn failure_receipt(&self, pid: u32) -> Option<String> {
        match self {
            Self::Failed(reason) => Some(format!(
                "process tree reclamation failed for pid {pid}: {reason}; descendants may still be running"
            )),
            Self::Reclaimed | Self::AlreadyGone | Self::Skipped => None,
        }
    }

    /// True when the outcome leaves no known survivor.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        !matches!(self, Self::Failed(_))
    }
}

/// Kill `pid` and every process it spawned.
///
/// Returns what was achieved. The caller should still `wait()` the direct child
/// it owns: this function signals the tree, it does not reap it.
#[must_use]
pub fn kill_process_tree(pid: u32) -> Reclamation {
    if pid == 0 {
        // pid 0 means "the caller's own group". Signalling it would target the
        // agent itself, so this is never an attempt.
        return Reclamation::Skipped;
    }

    #[cfg(windows)]
    let (program, args) = {
        // /T walks the child tree, /F terminates without prompting.
        ("taskkill", vec!["/F".to_string(), "/T".to_string(), "/PID".to_string(), pid.to_string()])
    };
    #[cfg(not(windows))]
    let (program, args) = {
        // A negative pid signals the whole process group; the child must have
        // been spawned with `prepare_process_group`. Without that, this fails
        // and the caller's own `Child::kill` is still what stops the child.
        ("kill", vec!["-9".to_string(), format!("-{pid}")])
    };

    run_kill_command(program, &args)
}

/// Map the result of a tree-kill command onto an outcome.
///
/// Split out from [`kill_process_tree`] so the mapping can be exercised against
/// commands whose outcome is known: the failure branch cannot otherwise be
/// reached from a test without signalling a real protected process, which is
/// not something a test suite should do.
fn run_kill_command(program: &str, args: &[String]) -> Reclamation {
    let output = match Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            return Reclamation::Failed(format!("could not run `{program}`: {error}"));
        }
    };

    if output.status.success() {
        return Reclamation::Reclaimed;
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let already_gone = {
        #[cfg(windows)]
        {
            // taskkill reports 128 when the pid is not found.
            output.status.code() == Some(128) || stderr.to_ascii_lowercase().contains("not found")
        }
        #[cfg(not(windows))]
        {
            stderr.contains("No such process")
        }
    };
    if already_gone {
        return Reclamation::AlreadyGone;
    }

    let code = output.status.code().map_or_else(|| "signal".to_string(), |code| code.to_string());
    Reclamation::Failed(if stderr.is_empty() {
        format!("`{program}` exited with {code}")
    } else {
        format!("`{program}` exited with {code}: {stderr}")
    })
}

/// Put a freshly spawned child into its own process group so
/// [`kill_process_tree`] can reach its descendants on Unix.
///
/// Windows needs no spawn-time setup (`taskkill /T` walks the tree), so this
/// is a no-op there.
pub fn prepare_process_group(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(not(unix))]
    {
        let _ = command;
    }
}

/// The recovery ledger for the current working directory.
///
/// Constructing this performs no I/O; the ledger is only touched when a call
/// is made to it.
#[must_use]
pub fn workspace_task_store() -> crate::active_task::ActiveTaskStore {
    match std::env::current_dir() {
        Ok(root) => crate::active_task::ActiveTaskStore::new(root),
        Err(_) => crate::active_task::ActiveTaskStore::new("."),
    }
}

/// Record a spawned process in the active task's recovery ledger.
///
/// Recording is conditional on a task already being active: the ledger exists
/// to make an interrupted task recoverable, and creating one for every tool
/// call would add files to a workspace that never asked for them. Failures are
/// ignored - the ledger is bookkeeping, never a gate on the work.
///
/// **A task is only active if something called `ActiveTaskStore::start_task`.**
/// Nothing in the product does that yet, so today this records nothing outside
/// tests; the wiring exists so that the spawn and exit paths are already
/// correct when an entry point is chosen. See the note at the end of this
/// module's documentation in the issue list (`DEV-CON-08`).
pub fn track_spawned_process(
    store: &crate::active_task::ActiveTaskStore,
    pid: u32,
    command: &str,
    purpose: &str,
) {
    if !store.has_active_task() || pid == 0 {
        return;
    }
    let cwd = std::env::current_dir().map(|path| path.display().to_string()).unwrap_or_default();
    let _ = store.track_process(pid, command, &cwd, None, purpose);
}

/// Mark a recorded process as stopped. Best-effort, like the recording.
pub fn untrack_finished_process(store: &crate::active_task::ActiveTaskStore, pid: u32) {
    if !store.has_active_task() || pid == 0 {
        return;
    }
    let _ = store.untrack_process(pid);
}

/// [`track_spawned_process`] against the current directory's ledger.
pub fn record_spawn(pid: u32, command: &str, purpose: &str) {
    track_spawned_process(&workspace_task_store(), pid, command, purpose);
}

/// [`untrack_finished_process`] against the current directory's ledger.
pub fn record_exit(pid: u32) {
    untrack_finished_process(&workspace_task_store(), pid);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn killing_an_already_exited_pid_is_not_an_error() {
        // A child that finished on its own must not turn reclamation into a
        // failure: the goal is "no orphans", not "the kill succeeded".
        let mut child = Command::new(if cfg!(windows) { "cmd" } else { "sh" })
            .args(if cfg!(windows) { vec!["/C", "exit 0"] } else { vec!["-c", "exit 0"] })
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn short-lived child");
        let pid = child.id();
        child.wait().expect("child exits");
        let outcome = kill_process_tree(pid);
        assert!(outcome.is_clean(), "already-exited pid must be tolerated, got {outcome:?}");
        assert_eq!(
            outcome.failure_receipt(pid),
            None,
            "a clean outcome must not produce a receipt"
        );
    }

    #[test]
    fn pid_zero_is_never_signalled() {
        // pid 0 would address the caller's own process group.
        let outcome = kill_process_tree(0);
        assert_eq!(outcome, Reclamation::Skipped);
        assert!(outcome.is_clean());
    }

    #[test]
    fn a_refused_kill_produces_a_receipt_naming_the_pid() {
        // A refused kill and a successful one used to be indistinguishable,
        // because the exit status was discarded. The receipt is what makes the
        // difference observable to an operator.
        let outcome =
            Reclamation::Failed("`kill` exited with 1: Operation not permitted".to_string());
        assert!(!outcome.is_clean());
        let receipt = outcome.failure_receipt(4242).expect("a failure must produce a receipt");
        assert!(receipt.contains("4242"), "the receipt must name the pid: {receipt}");
        assert!(
            receipt.contains("Operation not permitted"),
            "the receipt must carry the reason: {receipt}"
        );
        assert!(
            receipt.contains("may still be running"),
            "the receipt must state the consequence, not just the error: {receipt}"
        );
    }

    #[test]
    fn only_a_failure_produces_a_receipt() {
        for outcome in [Reclamation::Reclaimed, Reclamation::AlreadyGone, Reclamation::Skipped] {
            assert_eq!(
                outcome.failure_receipt(7),
                None,
                "{outcome:?} is not a failure and must stay quiet"
            );
        }
    }

    #[test]
    fn killing_a_live_tree_is_reported_as_reclaimed() {
        let mut child = Command::new(if cfg!(windows) { "cmd" } else { "sh" })
            .args(if cfg!(windows) {
                vec!["/C", "ping -n 30 127.0.0.1 > nul"]
            } else {
                vec!["-c", "sleep 30"]
            })
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn long-lived child");
        let pid = child.id();

        let outcome = kill_process_tree(pid);
        let _ = child.wait();
        assert_eq!(
            outcome,
            Reclamation::Reclaimed,
            "a live child must be reported as reclaimed, not inferred from the absence of an error"
        );
    }

    /// Spawn `sh -c 'sleep 30 & echo $!'` into its own group and return the
    /// grandchild's pid, which only a tree kill can reach.
    #[cfg(unix)]
    fn spawn_grandchild() -> (std::process::Child, u32) {
        use std::io::BufRead as _;

        let mut command = Command::new("sh");
        command
            // The grandchild's own stdio is redirected away from our pipe.
            // Inheriting it would keep the pipe open after `sh` exits, so
            // reading the shell's output would block until the sleep finished -
            // which is both slow and defeats the test, since by then there is
            // nothing left to reclaim. (The Windows case had the same bug in
            // PowerShell form; that one is fixed by reading a single line.)
            .args(["-c", "sleep 30 >/dev/null 2>&1 & echo $!"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        prepare_process_group(&mut command);
        let mut child = command.spawn().expect("spawn tree");
        let stdout = child.stdout.take().expect("stdout is piped");
        let mut line = String::new();
        std::io::BufReader::new(stdout).read_line(&mut line).expect("read grandchild pid");
        let grandchild: u32 = line.trim().parse().expect("a pid");
        (child, grandchild)
    }

    #[cfg(unix)]
    fn process_exists(pid: u32) -> bool {
        // `kill -0` only tests for existence; it does not signal.
        Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    #[cfg(unix)]
    #[test]
    fn a_timeout_reclaims_the_grandchild_by_its_own_pid() {
        // The earlier version of this test scanned `ps` for "sleep 10" and
        // asserted on the first iteration, which could pass before the signal
        // landed and could match an unrelated process. Tracking the real pid is
        // deterministic.
        let (mut child, grandchild) = spawn_grandchild();
        let parent = child.id();
        assert!(process_exists(grandchild), "the grandchild should be running before the kill");

        let outcome = kill_process_tree(parent);
        let _ = child.wait();
        assert_eq!(outcome, Reclamation::Reclaimed);

        let started = Instant::now();
        while process_exists(grandchild) {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "grandchild {grandchild} survived a tree kill of its group"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_timeout_reclaims_the_grandchild_on_windows_too() {
        // Windows is where the two platforms diverge most (no process groups,
        // `taskkill /T` instead), and it had no tree test at all. PowerShell
        // reports the pid of the process it starts, so the grandchild can be
        // checked directly rather than inferred.
        // Read one line, not to EOF: powershell stays alive until the
        // grandchild exits, so reading to EOF would block for the whole ping
        // and then find nothing left to kill.
        use std::io::BufRead as _;

        let script = "$p = Start-Process -PassThru -WindowStyle Hidden \
                      -FilePath cmd -ArgumentList '/C','ping -n 30 127.0.0.1 > nul'; \
                      Write-Output $p.Id; $p.WaitForExit()";
        let mut child = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn powershell");
        let parent = child.id();
        let stdout = child.stdout.take().expect("stdout is piped");
        let mut line = String::new();
        std::io::BufReader::new(stdout).read_line(&mut line).expect("read grandchild pid");
        let grandchild: u32 = line
            .split_whitespace()
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| panic!("powershell did not report a pid, got {line:?}"));

        // `tasklist` is the portable existence check here; it ships with
        // Windows and needs no new dependency.
        let exists = |pid: u32| {
            Command::new("tasklist")
                .args(["/FI", &format!("PID eq {pid}"), "/NH"])
                .output()
                .is_ok_and(|output| {
                    String::from_utf8_lossy(&output.stdout).contains(&pid.to_string())
                })
        };
        assert!(exists(grandchild), "grandchild {grandchild} should be running before the kill");

        let outcome = kill_process_tree(parent);
        let _ = child.wait();
        assert!(outcome.is_clean(), "tree kill reported {outcome:?}");

        let started = Instant::now();
        while exists(grandchild) {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "grandchild {grandchild} survived a tree kill of its parent"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    #[test]
    fn a_command_that_fails_is_reported_as_failed_with_its_reason() {
        // The end-to-end failure branch, driven through a command whose failure
        // is known. Signalling a real protected process to reach this branch
        // would be a test suite doing something it has no business doing.
        #[cfg(windows)]
        let (program, args) =
            ("cmd", vec!["/C".to_string(), "echo access is denied 1>&2 & exit 3".to_string()]);
        #[cfg(not(windows))]
        let (program, args) = (
            "sh",
            vec!["-c".to_string(), "echo 'Operation not permitted' >&2; exit 3".to_string()],
        );

        match run_kill_command(program, &args) {
            Reclamation::Failed(reason) => {
                assert!(reason.contains('3'), "the exit code must be in the reason: {reason}");
                assert!(
                    reason.to_ascii_lowercase().contains("denied")
                        || reason.contains("Operation not permitted"),
                    "the command's own message must be carried through: {reason}"
                );
            }
            other => panic!("a failing kill command must not be reported as {other:?}"),
        }
    }

    #[test]
    fn a_command_that_succeeds_is_reported_as_reclaimed() {
        #[cfg(windows)]
        let (program, args) = ("cmd", vec!["/C".to_string(), "exit 0".to_string()]);
        #[cfg(not(windows))]
        let (program, args) = ("sh", vec!["-c".to_string(), "exit 0".to_string()]);
        assert_eq!(run_kill_command(program, &args), Reclamation::Reclaimed);
    }

    #[test]
    fn a_target_that_is_already_gone_is_not_a_failure() {
        // The signal is refused because there is nothing to signal. That is the
        // normal case at the end of a short-lived command, and it must not
        // produce a receipt or the message would become noise.
        #[cfg(windows)]
        let (program, args) =
            ("cmd", vec!["/C".to_string(), "echo ERROR: not found 1>&2 & exit 128".to_string()]);
        #[cfg(not(windows))]
        let (program, args) =
            ("sh", vec!["-c".to_string(), "echo 'kill: No such process' >&2; exit 1".to_string()]);

        let outcome = run_kill_command(program, &args);
        assert_eq!(
            outcome,
            Reclamation::AlreadyGone,
            "a refused signal for a missing target is not a reclamation failure"
        );
        assert_eq!(outcome.failure_receipt(9), None);
    }

    #[test]
    fn a_missing_program_is_a_failure_not_a_silent_pass() {
        let outcome = run_kill_command("this-program-does-not-exist-sego-test", &[]);
        match outcome {
            Reclamation::Failed(reason) => {
                assert!(reason.contains("could not run"), "unexpected reason: {reason}");
            }
            other => panic!("a missing kill program must be reported as a failure, got {other:?}"),
        }
    }

    #[test]
    fn the_ledger_records_a_spawn_only_while_a_task_is_active() {
        let root = std::env::temp_dir().join(format!(
            "process-tree-ledger-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time after epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("create temp root");
        let store = crate::active_task::ActiveTaskStore::new(&root);

        // No task yet: the spawn must not create ledger files in a workspace
        // that never asked for them.
        record_into(&store, 4242);
        assert!(
            !store.has_active_task(),
            "a spawn must not bring a task into existence on its own"
        );

        // With a task active, the same call records the process.
        store
            .start_task("task-ledger-test", "prove the wiring", None, None, None, Vec::new())
            .expect("start a task");
        assert!(store.has_active_task());
        record_into(&store, 4242);
        let tracked = |store: &crate::active_task::ActiveTaskStore| {
            store
                .load_task()
                .expect("read the ledger")
                .running_processes
                .into_iter()
                .find(|entry| entry.pid == 4242)
        };
        let entry = tracked(&store).expect("the spawn should be in the ledger");
        assert_eq!(entry.status, "running", "a fresh spawn is running");
        assert_eq!(entry.purpose, "test", "the purpose is recorded for the operator");

        // Exiting marks it stopped. The entry is kept rather than removed: the
        // ledger is a record of what ran, not only of what is running.
        untrack_finished_process(&store, 4242);
        let entry = tracked(&store).expect("the entry should remain as history");
        assert_eq!(entry.status, "stopped", "the exit should mark the process stopped");

        let _ = std::fs::remove_dir_all(&root);
    }

    fn record_into(store: &crate::active_task::ActiveTaskStore, pid: u32) {
        track_spawned_process(store, pid, "a-command", "test");
    }
}
