use std::io;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

pub(crate) fn is_literal_command(command: &str) -> bool {
    !command.starts_with("./") && !command.starts_with("../") && !Path::new(command).is_absolute()
}

pub(crate) fn plugin_command(command: &str) -> io::Result<Command> {
    if is_literal_command(command) {
        return Ok(literal_command(command));
    }

    let path = Path::new(command);
    if !path.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("plugin command path `{}` does not exist or is not a file", path.display()),
        ));
    }

    path_command(path)
}

#[cfg(windows)]
fn literal_command(command: &str) -> Command {
    let mut process = Command::new("cmd.exe");
    process.args(["/D", "/S", "/C"]).arg(command);
    process
}

#[cfg(not(windows))]
fn literal_command(command: &str) -> Command {
    let mut process = Command::new("sh");
    process.arg("-lc").arg(command);
    process
}

#[cfg(windows)]
fn path_command(path: &Path) -> io::Result<Command> {
    let extension = path.extension().and_then(|value| value.to_str()).map(str::to_ascii_lowercase);

    let process = match extension.as_deref() {
        Some("cmd" | "bat") => {
            let mut process = Command::new("cmd.exe");
            process.args(["/D", "/S", "/C"]).raw_arg(format!("\"\"{}\"\"", path.display()));
            process
        }
        Some("ps1") => {
            let mut process = Command::new("powershell.exe");
            process.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"]).arg(path);
            process
        }
        Some("sh") => {
            let mut process = Command::new("sh");
            process.arg(path);
            process
        }
        Some("exe" | "com") => Command::new(path),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "unsupported Windows plugin command `{}`; expected .cmd, .bat, .ps1, .sh, .exe, or .com",
                    path.display()
                ),
            ));
        }
    };

    Ok(process)
}

#[cfg(not(windows))]
fn path_command(path: &Path) -> io::Result<Command> {
    let mut process = Command::new("sh");
    process.arg(path);
    Ok(process)
}

#[cfg(test)]
mod tests {
    use super::{is_literal_command, plugin_command};
    use std::ffi::OsStr;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("plugins-command-runner-{label}-{nanos}"))
    }

    fn write_file(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent directory");
        }
        fs::write(path, "probe").expect("probe file");
    }

    #[test]
    fn classifies_literal_and_path_commands_without_file_associations() {
        assert!(is_literal_command("echo hello"));
        assert!(!is_literal_command("./hooks/pre.sh"));
        assert!(!is_literal_command("../hooks/pre.sh"));
    }

    #[test]
    fn missing_path_fails_closed_before_spawn() {
        let root = temp_dir("missing");
        let missing = root.join("missing.sh");
        let error = plugin_command(missing.to_str().expect("utf8 path"))
            .expect_err("missing path must fail closed");

        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    }

    #[cfg(windows)]
    #[test]
    fn windows_path_commands_select_explicit_synchronous_runners() {
        let root = temp_dir("windows-runners");
        let cmd_path = root.join("probe.cmd");
        let ps1_path = root.join("probe.ps1");
        let sh_path = root.join("probe.sh");
        let unsupported_path = root.join("probe.txt");
        for path in [&cmd_path, &ps1_path, &sh_path, &unsupported_path] {
            write_file(path);
        }

        let cmd = plugin_command(cmd_path.to_str().expect("utf8 path")).expect("cmd runner");
        let ps1 = plugin_command(ps1_path.to_str().expect("utf8 path")).expect("ps1 runner");
        let sh = plugin_command(sh_path.to_str().expect("utf8 path")).expect("sh runner");
        let unsupported = plugin_command(unsupported_path.to_str().expect("utf8 path"))
            .expect_err("unknown extensions must fail closed");

        assert_eq!(cmd.get_program(), OsStr::new("cmd.exe"));
        assert_eq!(ps1.get_program(), OsStr::new("powershell.exe"));
        assert_eq!(sh.get_program(), OsStr::new("sh"));
        assert_eq!(unsupported.kind(), std::io::ErrorKind::Unsupported);

        let _ = fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[test]
    fn windows_cmd_paths_with_spaces_execute_synchronously() {
        let root = temp_dir("windows path with spaces");
        let script_path = root.join("probe script.cmd");
        fs::create_dir_all(&root).expect("probe directory");
        fs::write(&script_path, "@echo off\r\necho synchronous\r\n").expect("probe command");

        let output = plugin_command(script_path.to_str().expect("utf8 path"))
            .expect("cmd runner")
            .output()
            .expect("cmd execution");

        assert!(
            output.status.success(),
            "status={} stdout={} stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "synchronous");
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_path_commands_use_sh_explicitly() {
        let root = temp_dir("unix-runner");
        let script_path = root.join("probe.sh");
        write_file(&script_path);

        let command =
            plugin_command(script_path.to_str().expect("utf8 path")).expect("shell runner");

        assert_eq!(command.get_program(), OsStr::new("sh"));
        let _ = fs::remove_dir_all(root);
    }
}
