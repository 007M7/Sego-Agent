use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum FilesystemIsolationMode {
    Off,
    #[default]
    WorkspaceOnly,
    AllowList,
}

impl FilesystemIsolationMode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::WorkspaceOnly => "workspace-only",
            Self::AllowList => "allow-list",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SandboxConfig {
    pub enabled: Option<bool>,
    pub namespace_restrictions: Option<bool>,
    pub network_isolation: Option<bool>,
    pub filesystem_mode: Option<FilesystemIsolationMode>,
    pub allowed_mounts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SandboxRequest {
    pub enabled: bool,
    pub namespace_restrictions: bool,
    pub network_isolation: bool,
    pub filesystem_mode: FilesystemIsolationMode,
    pub allowed_mounts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ContainerEnvironment {
    pub in_container: bool,
    pub markers: Vec<String>,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SandboxStatus {
    pub enabled: bool,
    pub requested: SandboxRequest,
    pub supported: bool,
    pub active: bool,
    pub namespace_supported: bool,
    pub namespace_active: bool,
    pub network_supported: bool,
    pub network_active: bool,
    pub filesystem_mode: FilesystemIsolationMode,
    pub filesystem_active: bool,
    pub allowed_mounts: Vec<String>,
    pub in_container: bool,
    pub container_markers: Vec<String>,
    /// What isolation this build provides, stated even when nothing was asked
    /// for. See [`PlatformIsolation`].
    pub platform_isolation: PlatformIsolation,
    pub fallback_reason: Option<String>,
}

/// Process isolation this build can provide, independent of any request.
///
/// The limitation used to be visible only through `fallback_reason`, which is
/// populated only when a caller asks for isolation, so a reader of the status
/// could conclude the platform isolates by default. Sego ships Windows, Linux
/// and macOS binaries while only Linux has an implementation; the
/// AppContainer/AppArmor material elsewhere in the repository is governance
/// evidence, not a runtime capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformIsolation {
    /// Linux namespaces via `unshare`.
    NamespaceIsolation,
    /// No equivalent implementation on this platform: the process runs
    /// unsandboxed however it is requested.
    Unavailable,
}

impl Default for PlatformIsolation {
    /// Absence of isolation, not a guess at its presence: a default that
    /// claimed `NamespaceIsolation` would overstate what an unconfigured build
    /// is doing.
    fn default() -> Self {
        Self::Unavailable
    }
}

impl PlatformIsolation {
    #[must_use]
    pub fn from_namespace_support(namespace_supported: bool) -> Self {
        if namespace_supported {
            Self::NamespaceIsolation
        } else {
            Self::Unavailable
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NamespaceIsolation => "namespace_isolation",
            Self::Unavailable => "unavailable",
        }
    }

    /// Human statement of the capability, so callers do not have to infer it.
    #[must_use]
    pub fn note(self) -> &'static str {
        match self {
            Self::NamespaceIsolation => "process isolation available: Linux namespaces via unshare",
            Self::Unavailable => {
                "no process isolation on this platform: Linux namespaces via unshare are the only \
                 implementation, so Windows and macOS run unsandboxed"
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxDetectionInputs<'a> {
    pub env_pairs: Vec<(String, String)>,
    pub dockerenv_exists: bool,
    pub containerenv_exists: bool,
    pub proc_1_cgroup: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxSandboxCommand {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl SandboxConfig {
    #[must_use]
    pub fn resolve_request(
        &self,
        enabled_override: Option<bool>,
        namespace_override: Option<bool>,
        network_override: Option<bool>,
        filesystem_mode_override: Option<FilesystemIsolationMode>,
        allowed_mounts_override: Option<Vec<String>>,
    ) -> SandboxRequest {
        SandboxRequest {
            enabled: enabled_override.unwrap_or(self.enabled.unwrap_or(true)),
            namespace_restrictions: namespace_override
                .unwrap_or(self.namespace_restrictions.unwrap_or(true)),
            network_isolation: network_override.unwrap_or(self.network_isolation.unwrap_or(false)),
            filesystem_mode: filesystem_mode_override.or(self.filesystem_mode).unwrap_or_default(),
            allowed_mounts: allowed_mounts_override.unwrap_or_else(|| self.allowed_mounts.clone()),
        }
    }
}

#[must_use]
pub fn detect_container_environment() -> ContainerEnvironment {
    let proc_1_cgroup = fs::read_to_string("/proc/1/cgroup").ok();
    detect_container_environment_from(SandboxDetectionInputs {
        env_pairs: env::vars().collect(),
        dockerenv_exists: Path::new("/.dockerenv").exists(),
        containerenv_exists: Path::new("/run/.containerenv").exists(),
        proc_1_cgroup: proc_1_cgroup.as_deref(),
    })
}

#[must_use]
pub fn detect_container_environment_from(
    inputs: SandboxDetectionInputs<'_>,
) -> ContainerEnvironment {
    let mut markers = Vec::new();
    if inputs.dockerenv_exists {
        markers.push("/.dockerenv".to_string());
    }
    if inputs.containerenv_exists {
        markers.push("/run/.containerenv".to_string());
    }
    for (key, value) in inputs.env_pairs {
        let normalized = key.to_ascii_lowercase();
        if matches!(
            normalized.as_str(),
            "container" | "docker" | "podman" | "kubernetes_service_host"
        ) && !value.is_empty()
        {
            markers.push(format!("env:{key}={value}"));
        }
    }
    if let Some(cgroup) = inputs.proc_1_cgroup {
        for needle in ["docker", "containerd", "kubepods", "podman", "libpod"] {
            if cgroup.contains(needle) {
                markers.push(format!("/proc/1/cgroup:{needle}"));
            }
        }
    }
    markers.sort();
    markers.dedup();
    ContainerEnvironment { in_container: !markers.is_empty(), markers }
}

#[must_use]
pub fn resolve_sandbox_status(config: &SandboxConfig, cwd: &Path) -> SandboxStatus {
    let request = config.resolve_request(None, None, None, None, None);
    resolve_sandbox_status_for_request(&request, cwd)
}

#[must_use]
pub fn resolve_sandbox_status_for_request(request: &SandboxRequest, cwd: &Path) -> SandboxStatus {
    let container = detect_container_environment();
    let namespace_supported = cfg!(target_os = "linux") && unshare_user_namespace_works();
    let network_supported = namespace_supported;
    let filesystem_active =
        request.enabled && request.filesystem_mode != FilesystemIsolationMode::Off;
    let mut fallback_reasons = Vec::new();

    if request.enabled && request.namespace_restrictions && !namespace_supported {
        fallback_reasons
            .push("namespace isolation unavailable (requires Linux with `unshare`)".to_string());
    }
    if request.enabled && request.network_isolation && !network_supported {
        fallback_reasons
            .push("network isolation unavailable (requires Linux with `unshare`)".to_string());
    }
    if request.enabled
        && request.filesystem_mode == FilesystemIsolationMode::AllowList
        && request.allowed_mounts.is_empty()
    {
        fallback_reasons
            .push("filesystem allow-list requested without configured mounts".to_string());
    }

    let active = request.enabled
        && (!request.namespace_restrictions || namespace_supported)
        && (!request.network_isolation || network_supported);

    let allowed_mounts = normalize_mounts(&request.allowed_mounts, cwd);

    SandboxStatus {
        enabled: request.enabled,
        requested: request.clone(),
        supported: namespace_supported,
        active,
        namespace_supported,
        namespace_active: request.enabled && request.namespace_restrictions && namespace_supported,
        network_supported,
        network_active: request.enabled && request.network_isolation && network_supported,
        filesystem_mode: request.filesystem_mode,
        filesystem_active,
        allowed_mounts,
        in_container: container.in_container,
        container_markers: container.markers,
        platform_isolation: PlatformIsolation::from_namespace_support(namespace_supported),
        fallback_reason: (!fallback_reasons.is_empty()).then(|| fallback_reasons.join("; ")),
    }
}

#[must_use]
pub fn build_linux_sandbox_command(
    command: &str,
    cwd: &Path,
    status: &SandboxStatus,
) -> Option<LinuxSandboxCommand> {
    if !cfg!(target_os = "linux")
        || !status.enabled
        || (!status.namespace_active && !status.network_active)
    {
        return None;
    }

    let mut args = vec![
        "--user".to_string(),
        "--map-root-user".to_string(),
        "--mount".to_string(),
        "--ipc".to_string(),
        "--pid".to_string(),
        "--uts".to_string(),
        "--fork".to_string(),
    ];
    if status.network_active {
        args.push("--net".to_string());
    }
    args.push("sh".to_string());
    args.push("-lc".to_string());
    args.push(command.to_string());

    let sandbox_home = cwd.join(".sandbox-home");
    let sandbox_tmp = cwd.join(".sandbox-tmp");
    let mut env = vec![
        ("HOME".to_string(), sandbox_home.display().to_string()),
        ("TMPDIR".to_string(), sandbox_tmp.display().to_string()),
        ("CLAWD_SANDBOX_FILESYSTEM_MODE".to_string(), status.filesystem_mode.as_str().to_string()),
        ("CLAWD_SANDBOX_ALLOWED_MOUNTS".to_string(), status.allowed_mounts.join(":")),
    ];
    if let Ok(path) = env::var("PATH") {
        env.push(("PATH".to_string(), path));
    }

    Some(LinuxSandboxCommand { program: "unshare".to_string(), args, env })
}

fn normalize_mounts(mounts: &[String], cwd: &Path) -> Vec<String> {
    let cwd = cwd.to_path_buf();
    mounts
        .iter()
        .map(|mount| {
            let path = PathBuf::from(mount);
            if path.is_absolute() {
                path
            } else {
                cwd.join(path)
            }
        })
        .map(|path| path.display().to_string())
        .collect()
}

fn command_exists(command: &str) -> bool {
    env::var_os("PATH")
        .is_some_and(|paths| env::split_paths(&paths).any(|path| path.join(command).exists()))
}

/// Check whether `unshare --user` actually works on this system.
/// On some CI environments (e.g. GitHub Actions), the binary exists but
/// user namespaces are restricted, causing silent failures.
fn unshare_user_namespace_works() -> bool {
    use std::sync::OnceLock;
    static RESULT: OnceLock<bool> = OnceLock::new();
    *RESULT.get_or_init(|| {
        if !command_exists("unshare") {
            return false;
        }
        std::process::Command::new("unshare")
            .args(["--user", "--map-root-user", "true"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

#[cfg(test)]
mod tests {
    use super::{
        build_linux_sandbox_command, detect_container_environment_from, resolve_sandbox_status,
        FilesystemIsolationMode, PlatformIsolation, SandboxConfig, SandboxDetectionInputs,
    };
    use std::path::Path;

    #[test]
    fn platform_isolation_is_declared_even_when_nothing_is_requested() {
        // The status has to state the platform capability by itself: a caller
        // that asks for no isolation must still be able to tell whether this
        // build could have provided it. Previously the gap appeared only in
        // `fallback_reason`, and only when isolation was requested.
        let status = resolve_sandbox_status(&SandboxConfig::default(), Path::new("."));

        assert_eq!(
            status.platform_isolation,
            PlatformIsolation::from_namespace_support(status.namespace_supported),
            "the declaration must agree with the per-request capability flag"
        );
        assert!(
            !status.platform_isolation.note().is_empty(),
            "the capability must carry a human statement"
        );
        assert_eq!(
            status.platform_isolation.as_str(),
            if status.namespace_supported { "namespace_isolation" } else { "unavailable" }
        );

        // The default config asks for namespace restrictions, so a fallback
        // reason is expected there. Turning the sandbox off is the case this
        // declaration exists for: nothing is requested, `fallback_reason` stays
        // empty, and the platform capability still has to be readable.
        let disabled = SandboxConfig { enabled: Some(false), ..SandboxConfig::default() };
        let quiet = resolve_sandbox_status(&disabled, Path::new("."));
        assert!(quiet.fallback_reason.is_none(), "nothing was requested");
        assert_eq!(
            quiet.platform_isolation, status.platform_isolation,
            "the declaration must not depend on whether isolation was requested"
        );

        #[cfg(not(target_os = "linux"))]
        {
            assert_eq!(quiet.platform_isolation, PlatformIsolation::Unavailable);
            assert!(
                quiet.platform_isolation.note().contains("no process isolation"),
                "a platform without an implementation must say so"
            );
        }
    }

    #[test]
    fn isolation_default_does_not_overclaim() {
        assert_eq!(PlatformIsolation::default(), PlatformIsolation::Unavailable);
        assert!(PlatformIsolation::Unavailable.note().contains("unsandboxed"));
        assert_eq!(
            PlatformIsolation::from_namespace_support(true),
            PlatformIsolation::NamespaceIsolation
        );
    }

    #[test]
    fn detects_container_markers_from_multiple_sources() {
        let detected = detect_container_environment_from(SandboxDetectionInputs {
            env_pairs: vec![("container".to_string(), "docker".to_string())],
            dockerenv_exists: true,
            containerenv_exists: false,
            proc_1_cgroup: Some("12:memory:/docker/abc"),
        });

        assert!(detected.in_container);
        assert!(detected.markers.iter().any(|marker| marker == "/.dockerenv"));
        assert!(detected.markers.iter().any(|marker| marker == "env:container=docker"));
        assert!(detected.markers.iter().any(|marker| marker == "/proc/1/cgroup:docker"));
    }

    #[test]
    fn resolves_request_with_overrides() {
        let config = SandboxConfig {
            enabled: Some(true),
            namespace_restrictions: Some(true),
            network_isolation: Some(false),
            filesystem_mode: Some(FilesystemIsolationMode::WorkspaceOnly),
            allowed_mounts: vec!["logs".to_string()],
        };

        let request = config.resolve_request(
            Some(true),
            Some(false),
            Some(true),
            Some(FilesystemIsolationMode::AllowList),
            Some(vec!["tmp".to_string()]),
        );

        assert!(request.enabled);
        assert!(!request.namespace_restrictions);
        assert!(request.network_isolation);
        assert_eq!(request.filesystem_mode, FilesystemIsolationMode::AllowList);
        assert_eq!(request.allowed_mounts, vec!["tmp"]);
    }

    #[test]
    fn builds_linux_launcher_with_network_flag_when_requested() {
        let config = SandboxConfig::default();
        let status = super::resolve_sandbox_status_for_request(
            &config.resolve_request(
                Some(true),
                Some(true),
                Some(true),
                Some(FilesystemIsolationMode::WorkspaceOnly),
                None,
            ),
            Path::new("/workspace"),
        );

        if let Some(launcher) =
            build_linux_sandbox_command("printf hi", Path::new("/workspace"), &status)
        {
            assert_eq!(launcher.program, "unshare");
            assert!(launcher.args.iter().any(|arg| arg == "--mount"));
            assert!(launcher.args.iter().any(|arg| arg == "--net") == status.network_active);
        }
    }
}
