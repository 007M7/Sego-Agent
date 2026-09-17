//! Finding, loading and installing the definition files behind the skills and
//! agents commands: skill root discovery, install from a path, frontmatter
//! parsing, and the loaders that turn roots into summaries.
//!
//! Moved out of `lib.rs` as one layer (DEV-STRUCT-01). Bodies are verbatim.
//!
//! These functions touch the filesystem and the environment (install roots come
//! from `CODEX_HOME` or the home directory), which is why they are kept apart
//! from the renderers that only format what they return.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::{AgentSummary, DefinitionSource, InstalledSkill, SkillOrigin, SkillRoot, SkillSummary};
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SkillInstallSource {
    Directory { root: PathBuf, prompt_path: PathBuf },
    MarkdownFile { path: PathBuf },
}

pub(crate) fn discover_definition_roots(
    cwd: &Path,
    leaf: &str,
) -> Vec<(DefinitionSource, PathBuf)> {
    let mut roots = Vec::new();

    for ancestor in cwd.ancestors() {
        push_unique_root(
            &mut roots,
            DefinitionSource::ProjectCodex,
            ancestor.join(".codex").join(leaf),
        );
        push_unique_root(
            &mut roots,
            DefinitionSource::ProjectClaude,
            ancestor.join(".claude").join(leaf),
        );
    }

    if let Ok(codex_home) = env::var("CODEX_HOME") {
        push_unique_root(
            &mut roots,
            DefinitionSource::UserCodexHome,
            PathBuf::from(codex_home).join(leaf),
        );
    }

    if let Some(home) = env::var_os("HOME") {
        let home = PathBuf::from(home);
        push_unique_root(&mut roots, DefinitionSource::UserCodex, home.join(".codex").join(leaf));
        push_unique_root(&mut roots, DefinitionSource::UserClaude, home.join(".claude").join(leaf));
    }

    roots
}

pub(crate) fn discover_skill_roots(cwd: &Path) -> Vec<SkillRoot> {
    let mut roots = Vec::new();

    for ancestor in cwd.ancestors() {
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::ProjectCodex,
            ancestor.join(".codex").join("skills"),
            SkillOrigin::SkillsDir,
        );
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::ProjectClaude,
            ancestor.join(".claude").join("skills"),
            SkillOrigin::SkillsDir,
        );
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::ProjectCodex,
            ancestor.join(".codex").join("commands"),
            SkillOrigin::LegacyCommandsDir,
        );
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::ProjectClaude,
            ancestor.join(".claude").join("commands"),
            SkillOrigin::LegacyCommandsDir,
        );
    }

    if let Ok(codex_home) = env::var("CODEX_HOME") {
        let codex_home = PathBuf::from(codex_home);
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::UserCodexHome,
            codex_home.join("skills"),
            SkillOrigin::SkillsDir,
        );
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::UserCodexHome,
            codex_home.join("commands"),
            SkillOrigin::LegacyCommandsDir,
        );
    }

    if let Some(home) = env::var_os("HOME") {
        let home = PathBuf::from(home);
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::UserCodex,
            home.join(".codex").join("skills"),
            SkillOrigin::SkillsDir,
        );
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::UserCodex,
            home.join(".codex").join("commands"),
            SkillOrigin::LegacyCommandsDir,
        );
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::UserClaude,
            home.join(".claude").join("skills"),
            SkillOrigin::SkillsDir,
        );
        push_unique_skill_root(
            &mut roots,
            DefinitionSource::UserClaude,
            home.join(".claude").join("commands"),
            SkillOrigin::LegacyCommandsDir,
        );
    }

    roots
}

pub(crate) fn install_skill(source: &str, cwd: &Path) -> std::io::Result<InstalledSkill> {
    let registry_root = default_skill_install_root()?;
    install_skill_into(source, cwd, &registry_root)
}

pub(crate) fn install_skill_into(
    source: &str,
    cwd: &Path,
    registry_root: &Path,
) -> std::io::Result<InstalledSkill> {
    let source = resolve_skill_install_source(source, cwd)?;
    let prompt_path = source.prompt_path();
    let contents = fs::read_to_string(prompt_path)?;
    let display_name = parse_skill_frontmatter(&contents).0;
    let invocation_name = derive_skill_install_name(&source, display_name.as_deref())?;
    let installed_path = registry_root.join(&invocation_name);

    if installed_path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!(
                "skill '{invocation_name}' is already installed at {}",
                installed_path.display()
            ),
        ));
    }

    fs::create_dir_all(&installed_path)?;
    let install_result = match &source {
        SkillInstallSource::Directory { root, .. } => {
            copy_directory_contents(root, &installed_path)
        }
        SkillInstallSource::MarkdownFile { path } => {
            fs::copy(path, installed_path.join("SKILL.md")).map(|_| ())
        }
    };
    if let Err(error) = install_result {
        let _ = fs::remove_dir_all(&installed_path);
        return Err(error);
    }

    Ok(InstalledSkill {
        invocation_name,
        display_name,
        source: source.report_path().to_path_buf(),
        registry_root: registry_root.to_path_buf(),
        installed_path,
    })
}

pub(crate) fn default_skill_install_root() -> std::io::Result<PathBuf> {
    if let Ok(codex_home) = env::var("CODEX_HOME") {
        return Ok(PathBuf::from(codex_home).join("skills"));
    }
    if let Some(home) = env::var_os("HOME") {
        return Ok(PathBuf::from(home).join(".codex").join("skills"));
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "unable to resolve a skills install root; set CODEX_HOME or HOME",
    ))
}

pub(crate) fn resolve_skill_install_source(
    source: &str,
    cwd: &Path,
) -> std::io::Result<SkillInstallSource> {
    let candidate = PathBuf::from(source);
    let source = if candidate.is_absolute() { candidate } else { cwd.join(candidate) };
    let source = fs::canonicalize(&source)?;

    if source.is_dir() {
        let prompt_path = source.join("SKILL.md");
        if !prompt_path.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("skill directory '{}' must contain SKILL.md", source.display()),
            ));
        }
        return Ok(SkillInstallSource::Directory { root: source, prompt_path });
    }

    if source.extension().is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("md")) {
        return Ok(SkillInstallSource::MarkdownFile { path: source });
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!(
            "skill source '{}' must be a directory with SKILL.md or a markdown file",
            source.display()
        ),
    ))
}

pub(crate) fn derive_skill_install_name(
    source: &SkillInstallSource,
    declared_name: Option<&str>,
) -> std::io::Result<String> {
    for candidate in [declared_name, source.fallback_name().as_deref()] {
        if let Some(candidate) = candidate.and_then(sanitize_skill_invocation_name) {
            return Ok(candidate);
        }
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!(
            "unable to derive an installable invocation name from '{}'",
            source.report_path().display()
        ),
    ))
}

pub(crate) fn sanitize_skill_invocation_name(candidate: &str) -> Option<String> {
    let trimmed = candidate.trim().trim_start_matches('/').trim_start_matches('$');
    if trimmed.is_empty() {
        return None;
    }

    let mut sanitized = String::new();
    let mut last_was_separator = false;
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            sanitized.push(ch.to_ascii_lowercase());
            last_was_separator = false;
        } else if (ch.is_whitespace() || matches!(ch, '/' | '\\'))
            && !last_was_separator
            && !sanitized.is_empty()
        {
            sanitized.push('-');
            last_was_separator = true;
        }
    }

    let sanitized = sanitized.trim_matches(|ch| matches!(ch, '-' | '_' | '.')).to_string();
    (!sanitized.is_empty()).then_some(sanitized)
}

pub(crate) fn copy_directory_contents(source: &Path, destination: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let entry_type = entry.file_type()?;
        let destination_path = destination.join(entry.file_name());
        if entry_type.is_dir() {
            fs::create_dir_all(&destination_path)?;
            copy_directory_contents(&entry.path(), &destination_path)?;
        } else {
            fs::copy(entry.path(), destination_path)?;
        }
    }
    Ok(())
}

pub(crate) fn push_unique_root(
    roots: &mut Vec<(DefinitionSource, PathBuf)>,
    source: DefinitionSource,
    path: PathBuf,
) {
    if path.is_dir() && !roots.iter().any(|(_, existing)| existing == &path) {
        roots.push((source, path));
    }
}

pub(crate) fn push_unique_skill_root(
    roots: &mut Vec<SkillRoot>,
    source: DefinitionSource,
    path: PathBuf,
    origin: SkillOrigin,
) {
    if path.is_dir() && !roots.iter().any(|existing| existing.path == path) {
        roots.push(SkillRoot { source, path, origin });
    }
}

pub(crate) fn load_agents_from_roots(
    roots: &[(DefinitionSource, PathBuf)],
) -> std::io::Result<Vec<AgentSummary>> {
    let mut agents = Vec::new();
    let mut active_sources = BTreeMap::<String, DefinitionSource>::new();

    for (source, root) in roots {
        let mut root_agents = Vec::new();
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if entry.path().extension().is_none_or(|ext| ext != "toml") {
                continue;
            }
            let contents = fs::read_to_string(entry.path())?;
            let fallback_name = entry.path().file_stem().map_or_else(
                || entry.file_name().to_string_lossy().to_string(),
                |stem| stem.to_string_lossy().to_string(),
            );
            root_agents.push(AgentSummary {
                name: parse_toml_string(&contents, "name").unwrap_or(fallback_name),
                description: parse_toml_string(&contents, "description"),
                model: parse_toml_string(&contents, "model"),
                reasoning_effort: parse_toml_string(&contents, "model_reasoning_effort"),
                source: *source,
                shadowed_by: None,
            });
        }
        root_agents.sort_by(|left, right| left.name.cmp(&right.name));

        for mut agent in root_agents {
            let key = agent.name.to_ascii_lowercase();
            if let Some(existing) = active_sources.get(&key) {
                agent.shadowed_by = Some(*existing);
            } else {
                active_sources.insert(key, agent.source);
            }
            agents.push(agent);
        }
    }

    Ok(agents)
}

pub(crate) fn load_skills_from_roots(roots: &[SkillRoot]) -> std::io::Result<Vec<SkillSummary>> {
    let mut skills = Vec::new();
    let mut active_sources = BTreeMap::<String, DefinitionSource>::new();

    for root in roots {
        let mut root_skills = Vec::new();
        for entry in fs::read_dir(&root.path)? {
            let entry = entry?;
            match root.origin {
                SkillOrigin::SkillsDir => {
                    if !entry.path().is_dir() {
                        continue;
                    }
                    let skill_path = entry.path().join("SKILL.md");
                    if !skill_path.is_file() {
                        continue;
                    }
                    let contents = fs::read_to_string(skill_path)?;
                    let (name, description) = parse_skill_frontmatter(&contents);
                    root_skills.push(SkillSummary {
                        name: name
                            .unwrap_or_else(|| entry.file_name().to_string_lossy().to_string()),
                        description,
                        source: root.source,
                        shadowed_by: None,
                        origin: root.origin,
                    });
                }
                SkillOrigin::LegacyCommandsDir => {
                    let path = entry.path();
                    let markdown_path = if path.is_dir() {
                        let skill_path = path.join("SKILL.md");
                        if !skill_path.is_file() {
                            continue;
                        }
                        skill_path
                    } else if path
                        .extension()
                        .is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("md"))
                    {
                        path
                    } else {
                        continue;
                    };

                    let contents = fs::read_to_string(&markdown_path)?;
                    let fallback_name = markdown_path.file_stem().map_or_else(
                        || entry.file_name().to_string_lossy().to_string(),
                        |stem| stem.to_string_lossy().to_string(),
                    );
                    let (name, description) = parse_skill_frontmatter(&contents);
                    root_skills.push(SkillSummary {
                        name: name.unwrap_or(fallback_name),
                        description,
                        source: root.source,
                        shadowed_by: None,
                        origin: root.origin,
                    });
                }
            }
        }
        root_skills.sort_by(|left, right| left.name.cmp(&right.name));

        for mut skill in root_skills {
            let key = skill.name.to_ascii_lowercase();
            if let Some(existing) = active_sources.get(&key) {
                skill.shadowed_by = Some(*existing);
            } else {
                active_sources.insert(key, skill.source);
            }
            skills.push(skill);
        }
    }

    Ok(skills)
}

pub(crate) fn parse_toml_string(contents: &str, key: &str) -> Option<String> {
    let prefix = format!("{key} =");
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        let Some(value) = trimmed.strip_prefix(&prefix) else {
            continue;
        };
        let value = value.trim();
        let Some(value) = value.strip_prefix('"').and_then(|value| value.strip_suffix('"')) else {
            continue;
        };
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

pub(crate) fn parse_skill_frontmatter(contents: &str) -> (Option<String>, Option<String>) {
    let mut lines = contents.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (None, None);
    }

    let mut name = None;
    let mut description = None;
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if let Some(value) = trimmed.strip_prefix("name:") {
            let value = unquote_frontmatter_value(value.trim());
            if !value.is_empty() {
                name = Some(value);
            }
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("description:") {
            let value = unquote_frontmatter_value(value.trim());
            if !value.is_empty() {
                description = Some(value);
            }
        }
    }

    (name, description)
}

pub(crate) fn unquote_frontmatter_value(value: &str) -> String {
    value
        .strip_prefix('"')
        .and_then(|trimmed| trimmed.strip_suffix('"'))
        .or_else(|| value.strip_prefix('\'').and_then(|trimmed| trimmed.strip_suffix('\'')))
        .unwrap_or(value)
        .trim()
        .to_string()
}
