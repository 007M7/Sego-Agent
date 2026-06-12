use std::path::Path;

use super::VerificationScope;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationCommand {
    pub label: String,
    pub working_dir: String,
    pub program: String,
    pub args: Vec<String>,
}

impl VerificationCommand {
    #[must_use]
    pub fn new(
        label: impl Into<String>,
        working_dir: impl Into<String>,
        program: impl Into<String>,
        args: Vec<String>,
    ) -> Self {
        Self {
            label: label.into(),
            working_dir: working_dir.into(),
            program: program.into(),
            args,
        }
    }

    #[must_use]
    pub fn display_command(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationPlanStatus {
    Ready,
    NoPlan { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationPlan {
    pub scope: VerificationScope,
    pub status: VerificationPlanStatus,
    pub commands: Vec<VerificationCommand>,
}

impl VerificationPlan {
    #[must_use]
    pub fn ready(scope: VerificationScope, commands: Vec<VerificationCommand>) -> Self {
        Self {
            scope,
            status: VerificationPlanStatus::Ready,
            commands,
        }
    }

    #[must_use]
    pub fn no_plan(scope: VerificationScope, reason: impl Into<String>) -> Self {
        Self {
            scope,
            status: VerificationPlanStatus::NoPlan {
                reason: reason.into(),
            },
            commands: Vec::new(),
        }
    }
}

#[must_use]
pub fn build_verification_plan(cwd: &Path, scope: VerificationScope) -> VerificationPlan {
    if cwd.join("Cargo.toml").is_file() {
        return build_rust_plan(scope, ".");
    }

    if cwd.join("rust").join("Cargo.toml").is_file() {
        return build_rust_plan(scope, "rust");
    }

    if cwd.join("package.json").is_file() {
        return build_node_plan(cwd, scope, ".");
    }

    VerificationPlan::no_plan(
        scope,
        "no supported project manifest found (expected Cargo.toml or package.json)",
    )
}

fn build_rust_plan(scope: VerificationScope, working_dir: &str) -> VerificationPlan {
    let mut commands = Vec::new();
    match scope {
        VerificationScope::Fast => {
            commands.push(VerificationCommand::new(
                "rust build",
                working_dir,
                "cargo",
                vec!["build".to_string()],
            ));
        }
        VerificationScope::Auto | VerificationScope::Full => {
            commands.push(VerificationCommand::new(
                "rust build",
                working_dir,
                "cargo",
                vec!["build".to_string()],
            ));
            commands.push(VerificationCommand::new(
                "rust tests",
                working_dir,
                "cargo",
                vec!["test".to_string()],
            ));
        }
    }
    VerificationPlan::ready(scope, commands)
}

fn build_node_plan(cwd: &Path, scope: VerificationScope, working_dir: &str) -> VerificationPlan {
    let package_json = std::fs::read_to_string(cwd.join("package.json")).unwrap_or_default();
    let has_test = package_json.contains("\"test\"");
    let has_build = package_json.contains("\"build\"");
    let package_manager = if cwd.join("pnpm-lock.yaml").is_file() {
        "pnpm"
    } else if cwd.join("yarn.lock").is_file() {
        "yarn"
    } else {
        "npm"
    };

    let mut commands = Vec::new();
    if has_test {
        commands.push(node_command(package_manager, "node tests", working_dir, "test"));
    }
    if matches!(scope, VerificationScope::Auto | VerificationScope::Full) && has_build {
        commands.push(node_command(package_manager, "node build", working_dir, "build"));
    }

    if commands.is_empty() {
        return VerificationPlan::no_plan(
            scope,
            "package.json found but no test/build scripts were detected",
        );
    }

    VerificationPlan::ready(scope, commands)
}

fn node_command(
    package_manager: &str,
    label: &str,
    working_dir: &str,
    script: &str,
) -> VerificationCommand {
    match package_manager {
        "pnpm" => VerificationCommand::new(label, working_dir, "pnpm", vec![script.to_string()]),
        "yarn" => VerificationCommand::new(label, working_dir, "yarn", vec![script.to_string()]),
        _ => VerificationCommand::new(
            label,
            working_dir,
            "npm",
            vec!["run".to_string(), script.to_string()],
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{build_verification_plan, VerificationPlanStatus};
    use crate::VerificationScope;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn rust_auto_plan_runs_build_and_tests() {
        let cwd = temp_dir("verify-rust");
        fs::write(cwd.join("Cargo.toml"), "[package]\nname='demo'\n").expect("write manifest");

        let plan = build_verification_plan(&cwd, VerificationScope::Auto);

        assert_eq!(plan.status, VerificationPlanStatus::Ready);
        assert_eq!(plan.commands.len(), 2);
        assert_eq!(plan.commands[0].working_dir, ".");
        assert_eq!(plan.commands[0].display_command(), "cargo build");
        assert_eq!(plan.commands[1].display_command(), "cargo test");

        let _ = fs::remove_dir_all(cwd);
    }

    #[test]
    fn node_fast_plan_runs_tests_only() {
        let cwd = temp_dir("verify-node");
        fs::write(
            cwd.join("package.json"),
            r#"{"scripts":{"test":"vitest","build":"vite build"}}"#,
        )
        .expect("write package json");

        let plan = build_verification_plan(&cwd, VerificationScope::Fast);

        assert_eq!(plan.status, VerificationPlanStatus::Ready);
        assert_eq!(plan.commands.len(), 1);
        assert_eq!(plan.commands[0].display_command(), "npm run test");

        let _ = fs::remove_dir_all(cwd);
    }

    #[test]
    fn detects_nested_rust_workspace() {
        let cwd = temp_dir("verify-rust-root");
        fs::create_dir_all(cwd.join("rust")).expect("create rust dir");
        fs::write(cwd.join("rust").join("Cargo.toml"), "[workspace]\n").expect("write manifest");

        let plan = build_verification_plan(&cwd, VerificationScope::Fast);

        assert_eq!(plan.status, VerificationPlanStatus::Ready);
        assert_eq!(plan.commands[0].working_dir, "rust");

        let _ = fs::remove_dir_all(cwd);
    }

    #[test]
    fn rust_fast_plan_runs_build_only() {
        let cwd = temp_dir("verify-rust-fast");
        fs::write(cwd.join("Cargo.toml"), "[package]\nname='demo'\n").expect("write manifest");

        let plan = build_verification_plan(&cwd, VerificationScope::Fast);

        assert_eq!(plan.status, VerificationPlanStatus::Ready);
        assert_eq!(plan.commands.len(), 1);
        assert_eq!(plan.commands[0].display_command(), "cargo build");

        let _ = fs::remove_dir_all(cwd);
    }

    #[test]
    fn no_manifest_returns_no_plan() {
        let cwd = temp_dir("verify-empty");

        let plan = build_verification_plan(&cwd, VerificationScope::Auto);

        assert!(matches!(plan.status, VerificationPlanStatus::NoPlan { .. }));

        let _ = fs::remove_dir_all(cwd);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "sego-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }
}
