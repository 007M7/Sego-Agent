use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::json::{JsonError, JsonValue};
use crate::usage::TokenUsage;

const SESSION_VERSION: u32 = 1;
const ROTATE_AFTER_BYTES: u64 = 256 * 1024;
const MAX_ROTATED_FILES: usize = 3;
static SESSION_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Speaker role associated with a persisted conversation message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

/// Structured message content stored inside a [`Session`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentBlock {
    Text { text: String },
    ToolUse { id: String, name: String, input: String },
    ToolResult { tool_use_id: String, tool_name: String, output: String, is_error: bool },
    Thinking { thinking: String, signature: Option<String> },
}

/// One conversation message with optional token-usage metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationMessage {
    pub role: MessageRole,
    pub blocks: Vec<ContentBlock>,
    pub usage: Option<TokenUsage>,
}

/// Metadata describing the latest compaction that summarized a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCompaction {
    pub count: u32,
    pub removed_message_count: usize,
    pub summary: String,
}

/// Provenance recorded when a session is forked from another session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFork {
    pub parent_session_id: String,
    pub branch_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionPersistence {
    path: PathBuf,
}

/// Persisted conversational state for the runtime and CLI session manager.
#[derive(Debug, Clone)]
pub struct Session {
    pub version: u32,
    pub session_id: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub messages: Vec<ConversationMessage>,
    pub compaction: Option<SessionCompaction>,
    pub fork: Option<SessionFork>,
    persistence: Option<SessionPersistence>,
}

impl PartialEq for Session {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version
            && self.session_id == other.session_id
            && self.created_at_ms == other.created_at_ms
            && self.updated_at_ms == other.updated_at_ms
            && self.messages == other.messages
            && self.compaction == other.compaction
            && self.fork == other.fork
    }
}

impl Eq for Session {}

/// Errors raised while loading, parsing, or saving sessions.
#[derive(Debug)]
pub enum SessionError {
    Io(std::io::Error),
    Json(JsonError),
    Format(String),
}

impl Display for SessionError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Json(error) => write!(f, "{error}"),
            Self::Format(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for SessionError {}

impl From<std::io::Error> for SessionError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<JsonError> for SessionError {
    fn from(value: JsonError) -> Self {
        Self::Json(value)
    }
}

impl Session {
    #[must_use]
    pub fn new() -> Self {
        let now = current_time_millis();
        Self {
            version: SESSION_VERSION,
            session_id: generate_session_id(),
            created_at_ms: now,
            updated_at_ms: now,
            messages: Vec::new(),
            compaction: None,
            fork: None,
            persistence: None,
        }
    }

    #[must_use]
    pub fn with_persistence_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.persistence = Some(SessionPersistence { path: path.into() });
        self
    }

    #[must_use]
    pub fn persistence_path(&self) -> Option<&Path> {
        self.persistence.as_ref().map(|value| value.path.as_path())
    }

    pub fn save_to_path(&self, path: impl AsRef<Path>) -> Result<(), SessionError> {
        let path = path.as_ref();
        let snapshot = self.render_jsonl_snapshot()?;
        rotate_session_file_if_needed(path)?;
        write_atomic(path, &snapshot)?;
        cleanup_rotated_logs(path)?;
        Ok(())
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, SessionError> {
        let path = path.as_ref();
        let contents = fs::read_to_string(path)?;
        let session = match JsonValue::parse(&contents) {
            Ok(value)
                if value.as_object().is_some_and(|object| object.contains_key("messages")) =>
            {
                Self::from_json(&value)?
            }
            Err(_) | Ok(_) => Self::from_jsonl(&contents)?,
        };
        Ok(session.with_persistence_path(path.to_path_buf()))
    }

    pub fn push_message(&mut self, message: ConversationMessage) -> Result<(), SessionError> {
        self.touch();
        self.messages.push(message);
        let persist_result = {
            let message_ref = self.messages.last().ok_or_else(|| {
                SessionError::Format("message was just pushed but missing".to_string())
            })?;
            self.append_persisted_message(message_ref)
        };
        if let Err(error) = persist_result {
            self.messages.pop();
            return Err(error);
        }
        Ok(())
    }

    pub fn push_user_text(&mut self, text: impl Into<String>) -> Result<(), SessionError> {
        self.push_message(ConversationMessage::user_text(text))
    }

    pub fn record_compaction(&mut self, summary: impl Into<String>, removed_message_count: usize) {
        self.touch();
        let count = self.compaction.as_ref().map_or(1, |value| value.count + 1);
        self.compaction =
            Some(SessionCompaction { count, removed_message_count, summary: summary.into() });
    }

    #[must_use]
    pub fn fork(&self, branch_name: Option<String>) -> Self {
        let now = current_time_millis();
        Self {
            version: self.version,
            session_id: generate_session_id(),
            created_at_ms: now,
            updated_at_ms: now,
            messages: self.messages.clone(),
            compaction: self.compaction.clone(),
            fork: Some(SessionFork {
                parent_session_id: self.session_id.clone(),
                branch_name: normalize_optional_string(branch_name),
            }),
            persistence: None,
        }
    }

    pub fn to_json(&self) -> Result<JsonValue, SessionError> {
        let mut object = BTreeMap::new();
        object.insert("version".to_string(), JsonValue::Number(i64::from(self.version)));
        object.insert("session_id".to_string(), JsonValue::String(self.session_id.clone()));
        object.insert(
            "created_at_ms".to_string(),
            JsonValue::Number(i64_from_u64(self.created_at_ms, "created_at_ms")?),
        );
        object.insert(
            "updated_at_ms".to_string(),
            JsonValue::Number(i64_from_u64(self.updated_at_ms, "updated_at_ms")?),
        );
        object.insert(
            "messages".to_string(),
            JsonValue::Array(self.messages.iter().map(ConversationMessage::to_json).collect()),
        );
        if let Some(compaction) = &self.compaction {
            object.insert("compaction".to_string(), compaction.to_json()?);
        }
        if let Some(fork) = &self.fork {
            object.insert("fork".to_string(), fork.to_json());
        }
        Ok(JsonValue::Object(object))
    }

    pub fn from_json(value: &JsonValue) -> Result<Self, SessionError> {
        let object = value
            .as_object()
            .ok_or_else(|| SessionError::Format("session must be an object".to_string()))?;
        let version = object
            .get("version")
            .and_then(JsonValue::as_i64)
            .ok_or_else(|| SessionError::Format("missing version".to_string()))?;
        let version = u32::try_from(version)
            .map_err(|_| SessionError::Format("version out of range".to_string()))?;
        let messages = object
            .get("messages")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| SessionError::Format("missing messages".to_string()))?
            .iter()
            .map(ConversationMessage::from_json)
            .collect::<Result<Vec<_>, _>>()?;
        let now = current_time_millis();
        let session_id = object
            .get("session_id")
            .and_then(JsonValue::as_str)
            .map_or_else(generate_session_id, ToOwned::to_owned);
        let created_at_ms = object
            .get("created_at_ms")
            .map(|value| required_u64_from_value(value, "created_at_ms"))
            .transpose()?
            .unwrap_or(now);
        let updated_at_ms = object
            .get("updated_at_ms")
            .map(|value| required_u64_from_value(value, "updated_at_ms"))
            .transpose()?
            .unwrap_or(created_at_ms);
        let compaction = object.get("compaction").map(SessionCompaction::from_json).transpose()?;
        let fork = object.get("fork").map(SessionFork::from_json).transpose()?;
        Ok(Self {
            version,
            session_id,
            created_at_ms,
            updated_at_ms,
            messages,
            compaction,
            fork,
            persistence: None,
        })
    }

    fn from_jsonl(contents: &str) -> Result<Self, SessionError> {
        let mut version = SESSION_VERSION;
        let mut session_id = None;
        let mut created_at_ms = None;
        let mut updated_at_ms = None;
        let mut messages = Vec::new();
        let mut compaction = None;
        let mut fork = None;

        for (line_number, raw_line) in contents.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }
            let value = JsonValue::parse(line).map_err(|error| {
                SessionError::Format(format!(
                    "invalid JSONL record at line {}: {}",
                    line_number + 1,
                    error
                ))
            })?;
            let object = value.as_object().ok_or_else(|| {
                SessionError::Format(format!(
                    "JSONL record at line {} must be an object",
                    line_number + 1
                ))
            })?;
            match object.get("type").and_then(JsonValue::as_str).ok_or_else(|| {
                SessionError::Format(format!(
                    "JSONL record at line {} missing type",
                    line_number + 1
                ))
            })? {
                "session_meta" => {
                    version = required_u32(object, "version")?;
                    session_id = Some(required_string(object, "session_id")?);
                    created_at_ms = Some(required_u64(object, "created_at_ms")?);
                    updated_at_ms = Some(required_u64(object, "updated_at_ms")?);
                    fork = object.get("fork").map(SessionFork::from_json).transpose()?;
                }
                "message" => {
                    let message_value = object.get("message").ok_or_else(|| {
                        SessionError::Format(format!(
                            "JSONL record at line {} missing message",
                            line_number + 1
                        ))
                    })?;
                    messages.push(ConversationMessage::from_json(message_value)?);
                }
                "compaction" => {
                    compaction =
                        Some(SessionCompaction::from_json(&JsonValue::Object(object.clone()))?);
                }
                other => {
                    return Err(SessionError::Format(format!(
                        "unsupported JSONL record type at line {}: {other}",
                        line_number + 1
                    )))
                }
            }
        }

        let now = current_time_millis();
        Ok(Self {
            version,
            session_id: session_id.unwrap_or_else(generate_session_id),
            created_at_ms: created_at_ms.unwrap_or(now),
            updated_at_ms: updated_at_ms.unwrap_or(created_at_ms.unwrap_or(now)),
            messages,
            compaction,
            fork,
            persistence: None,
        })
    }

    fn render_jsonl_snapshot(&self) -> Result<String, SessionError> {
        let mut lines = vec![self.meta_record()?.render()];
        if let Some(compaction) = &self.compaction {
            lines.push(compaction.to_jsonl_record()?.render());
        }
        lines.extend(self.messages.iter().map(|message| message_record(message).render()));
        let mut rendered = lines.join("\n");
        rendered.push('\n');
        Ok(rendered)
    }

    fn append_persisted_message(&self, message: &ConversationMessage) -> Result<(), SessionError> {
        let Some(path) = self.persistence_path() else {
            return Ok(());
        };

        let needs_bootstrap = !path.exists() || fs::metadata(path)?.len() == 0;
        if needs_bootstrap {
            self.save_to_path(path)?;
            return Ok(());
        }

        let mut file = OpenOptions::new().append(true).open(path)?;
        writeln!(file, "{}", message_record(message).render())?;
        // Heal a transcript created before this was owner-only, rather than
        // leaving it readable for the rest of its life.
        let _ = crate::oauth::restrict_file_permissions(path);
        Ok(())
    }

    fn meta_record(&self) -> Result<JsonValue, SessionError> {
        let mut object = BTreeMap::new();
        object.insert("type".to_string(), JsonValue::String("session_meta".to_string()));
        object.insert("version".to_string(), JsonValue::Number(i64::from(self.version)));
        object.insert("session_id".to_string(), JsonValue::String(self.session_id.clone()));
        object.insert(
            "created_at_ms".to_string(),
            JsonValue::Number(i64_from_u64(self.created_at_ms, "created_at_ms")?),
        );
        object.insert(
            "updated_at_ms".to_string(),
            JsonValue::Number(i64_from_u64(self.updated_at_ms, "updated_at_ms")?),
        );
        if let Some(fork) = &self.fork {
            object.insert("fork".to_string(), fork.to_json());
        }
        Ok(JsonValue::Object(object))
    }

    fn touch(&mut self) {
        self.updated_at_ms = current_time_millis();
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl ConversationMessage {
    #[must_use]
    pub fn user_text(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            blocks: vec![ContentBlock::Text { text: text.into() }],
            usage: None,
        }
    }

    #[must_use]
    pub fn assistant(blocks: Vec<ContentBlock>) -> Self {
        Self { role: MessageRole::Assistant, blocks, usage: None }
    }

    #[must_use]
    pub fn assistant_with_usage(blocks: Vec<ContentBlock>, usage: Option<TokenUsage>) -> Self {
        Self { role: MessageRole::Assistant, blocks, usage }
    }

    #[must_use]
    pub fn tool_result(
        tool_use_id: impl Into<String>,
        tool_name: impl Into<String>,
        output: impl Into<String>,
        is_error: bool,
    ) -> Self {
        Self {
            role: MessageRole::Tool,
            blocks: vec![ContentBlock::ToolResult {
                tool_use_id: tool_use_id.into(),
                tool_name: tool_name.into(),
                output: output.into(),
                is_error,
            }],
            usage: None,
        }
    }

    #[must_use]
    pub fn to_json(&self) -> JsonValue {
        let mut object = BTreeMap::new();
        object.insert(
            "role".to_string(),
            JsonValue::String(
                match self.role {
                    MessageRole::System => "system",
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                    MessageRole::Tool => "tool",
                }
                .to_string(),
            ),
        );
        object.insert(
            "blocks".to_string(),
            JsonValue::Array(self.blocks.iter().map(ContentBlock::to_json).collect()),
        );
        if let Some(usage) = self.usage {
            object.insert("usage".to_string(), usage_to_json(usage));
        }
        JsonValue::Object(object)
    }

    fn from_json(value: &JsonValue) -> Result<Self, SessionError> {
        let object = value
            .as_object()
            .ok_or_else(|| SessionError::Format("message must be an object".to_string()))?;
        let role = match object
            .get("role")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| SessionError::Format("missing role".to_string()))?
        {
            "system" => MessageRole::System,
            "user" => MessageRole::User,
            "assistant" => MessageRole::Assistant,
            "tool" => MessageRole::Tool,
            other => {
                return Err(SessionError::Format(format!("unsupported message role: {other}")))
            }
        };
        let blocks = object
            .get("blocks")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| SessionError::Format("missing blocks".to_string()))?
            .iter()
            .map(ContentBlock::from_json)
            .collect::<Result<Vec<_>, _>>()?;
        let usage = object.get("usage").map(usage_from_json).transpose()?;
        Ok(Self { role, blocks, usage })
    }
}

impl ContentBlock {
    #[must_use]
    pub fn to_json(&self) -> JsonValue {
        let mut object = BTreeMap::new();
        match self {
            Self::Text { text } => {
                object.insert("type".to_string(), JsonValue::String("text".to_string()));
                object.insert("text".to_string(), JsonValue::String(text.clone()));
            }
            Self::ToolUse { id, name, input } => {
                object.insert("type".to_string(), JsonValue::String("tool_use".to_string()));
                object.insert("id".to_string(), JsonValue::String(id.clone()));
                object.insert("name".to_string(), JsonValue::String(name.clone()));
                object.insert("input".to_string(), JsonValue::String(input.clone()));
            }
            Self::ToolResult { tool_use_id, tool_name, output, is_error } => {
                object.insert("type".to_string(), JsonValue::String("tool_result".to_string()));
                object.insert("tool_use_id".to_string(), JsonValue::String(tool_use_id.clone()));
                object.insert("tool_name".to_string(), JsonValue::String(tool_name.clone()));
                object.insert("output".to_string(), JsonValue::String(output.clone()));
                object.insert("is_error".to_string(), JsonValue::Bool(*is_error));
            }
            Self::Thinking { thinking, signature } => {
                object.insert("type".to_string(), JsonValue::String("thinking".to_string()));
                object.insert("thinking".to_string(), JsonValue::String(thinking.clone()));
                if let Some(ref sig) = signature {
                    object.insert("signature".to_string(), JsonValue::String(sig.clone()));
                }
            }
        }
        JsonValue::Object(object)
    }

    fn from_json(value: &JsonValue) -> Result<Self, SessionError> {
        let object = value
            .as_object()
            .ok_or_else(|| SessionError::Format("block must be an object".to_string()))?;
        match object
            .get("type")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| SessionError::Format("missing block type".to_string()))?
        {
            "text" => Ok(Self::Text { text: required_string(object, "text")? }),
            "tool_use" => Ok(Self::ToolUse {
                id: required_string(object, "id")?,
                name: required_string(object, "name")?,
                input: required_string(object, "input")?,
            }),
            "tool_result" => Ok(Self::ToolResult {
                tool_use_id: required_string(object, "tool_use_id")?,
                tool_name: required_string(object, "tool_name")?,
                output: required_string(object, "output")?,
                is_error: object
                    .get("is_error")
                    .and_then(JsonValue::as_bool)
                    .ok_or_else(|| SessionError::Format("missing is_error".to_string()))?,
            }),
            "thinking" => Ok(Self::Thinking {
                thinking: object
                    .get("thinking")
                    .and_then(JsonValue::as_str)
                    .map(ToOwned::to_owned)
                    .unwrap_or_default(),
                signature: object
                    .get("signature")
                    .and_then(JsonValue::as_str)
                    .map(ToOwned::to_owned),
            }),
            other => Err(SessionError::Format(format!("unsupported block type: {other}"))),
        }
    }
}

impl SessionCompaction {
    pub fn to_json(&self) -> Result<JsonValue, SessionError> {
        let mut object = BTreeMap::new();
        object.insert("count".to_string(), JsonValue::Number(i64::from(self.count)));
        object.insert(
            "removed_message_count".to_string(),
            JsonValue::Number(i64_from_usize(self.removed_message_count, "removed_message_count")?),
        );
        object.insert("summary".to_string(), JsonValue::String(self.summary.clone()));
        Ok(JsonValue::Object(object))
    }

    pub fn to_jsonl_record(&self) -> Result<JsonValue, SessionError> {
        let mut object = BTreeMap::new();
        object.insert("type".to_string(), JsonValue::String("compaction".to_string()));
        object.insert("count".to_string(), JsonValue::Number(i64::from(self.count)));
        object.insert(
            "removed_message_count".to_string(),
            JsonValue::Number(i64_from_usize(self.removed_message_count, "removed_message_count")?),
        );
        object.insert("summary".to_string(), JsonValue::String(self.summary.clone()));
        Ok(JsonValue::Object(object))
    }

    fn from_json(value: &JsonValue) -> Result<Self, SessionError> {
        let object = value
            .as_object()
            .ok_or_else(|| SessionError::Format("compaction must be an object".to_string()))?;
        Ok(Self {
            count: required_u32(object, "count")?,
            removed_message_count: required_usize(object, "removed_message_count")?,
            summary: required_string(object, "summary")?,
        })
    }
}

impl SessionFork {
    #[must_use]
    pub fn to_json(&self) -> JsonValue {
        let mut object = BTreeMap::new();
        object.insert(
            "parent_session_id".to_string(),
            JsonValue::String(self.parent_session_id.clone()),
        );
        if let Some(branch_name) = &self.branch_name {
            object.insert("branch_name".to_string(), JsonValue::String(branch_name.clone()));
        }
        JsonValue::Object(object)
    }

    fn from_json(value: &JsonValue) -> Result<Self, SessionError> {
        let object = value
            .as_object()
            .ok_or_else(|| SessionError::Format("fork metadata must be an object".to_string()))?;
        Ok(Self {
            parent_session_id: required_string(object, "parent_session_id")?,
            branch_name: object
                .get("branch_name")
                .and_then(JsonValue::as_str)
                .map(ToOwned::to_owned),
        })
    }
}

fn message_record(message: &ConversationMessage) -> JsonValue {
    let mut object = BTreeMap::new();
    object.insert("type".to_string(), JsonValue::String("message".to_string()));
    object.insert("message".to_string(), redact_record(message.to_json()));
    JsonValue::Object(object)
}

/// Removes credential-shaped text from a record before it is written to disk.
///
/// This is a **best-effort filter, not a guarantee**. It removes values whose
/// shape is a known credential format, and values assigned to a
/// credential-named key that also look generated. It cannot recognise an
/// arbitrary secret that matches no known shape, so a transcript must never be
/// treated as safe-to-share merely because it passed through here.
///
/// Redaction happens at the persistence boundary only. The in-memory session is
/// left untouched, so the turn in progress still sees what the user typed.
///
/// `thinking` blocks are redacted but their `signature` is not: the signature
/// is opaque to us and is replayed back to the provider, so removing it would
/// break resume without protecting anything the provider did not already hold.
fn redact_record(value: JsonValue) -> JsonValue {
    match value {
        JsonValue::Object(object) => JsonValue::Object(
            object
                .into_iter()
                .map(|(key, entry)| {
                    let redacted = if REDACTABLE_RECORD_KEYS.contains(&key.as_str()) {
                        redact_credentials_in_value(entry)
                    } else {
                        redact_record(entry)
                    };
                    (key, redacted)
                })
                .collect(),
        ),
        JsonValue::Array(items) => JsonValue::Array(items.into_iter().map(redact_record).collect()),
        other => other,
    }
}

fn redact_credentials_in_value(value: JsonValue) -> JsonValue {
    match value {
        JsonValue::String(text) => JsonValue::String(redact_credentials(&text)),
        other => redact_record(other),
    }
}

/// Keys whose string values carry conversation content rather than structure.
const REDACTABLE_RECORD_KEYS: &[&str] = &["text", "input", "output", "thinking"];

/// Rewrites credential-shaped substrings as `[redacted:<kind>]`.
///
/// The replacement never introduces a `"` or a backslash, so the result stays
/// valid JSON when it is spliced into an already-serialized record. Running it
/// twice gives the same result as running it once, so a resumed transcript can
/// be saved again without accumulating markers.
fn redact_credentials(text: &str) -> String {
    let mut result = text.to_string();
    for pattern in credential_patterns() {
        let kind = pattern.kind;
        result = pattern
            .regex
            .replace_all(&result, |captures: &regex::Captures<'_>| match pattern.redaction {
                Redaction::WholeMatch => format!("[redacted:{kind}]"),
                Redaction::KeepFirstGroup => {
                    format!("{}[redacted:{kind}]", &captures[1])
                }
                Redaction::KeepPrefixIfValueLooksGenerated => {
                    let matched = captures.get(0).map_or("", |group| group.as_str());
                    let value = captures.get(3).map_or("", |group| group.as_str());
                    if value.bytes().any(|byte| byte.is_ascii_digit()) {
                        format!("{}{}[redacted:{kind}]", &captures[1], &captures[2])
                    } else {
                        matched.to_string()
                    }
                }
            })
            .into_owned();
    }
    result
}

/// How much of a match survives redaction.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Redaction {
    /// The whole match is a secret.
    WholeMatch,
    /// Group 1 is context worth keeping (a scheme name, a key name).
    KeepFirstGroup,
    /// Groups 1 and 2 are the name and separator; group 3 is the value, and it
    /// is only treated as a secret when it also looks generated.
    KeepPrefixIfValueLooksGenerated,
}

struct CredentialPattern {
    regex: regex::Regex,
    kind: &'static str,
    redaction: Redaction,
}

fn credential_patterns() -> &'static [CredentialPattern] {
    static PATTERNS: std::sync::OnceLock<Vec<CredentialPattern>> = std::sync::OnceLock::new();
    PATTERNS.get_or_init(|| {
        // Ordered: multi-line and prefix-anchored shapes first, then the looser
        // assignment rule, so a token removed by an earlier pass is not
        // re-examined by a later one.
        let sources: &[(&str, &str, Redaction)] = &[
            (
                r"(?s)-----BEGIN[ A-Z]*PRIVATE KEY-----.*?-----END[ A-Z]*PRIVATE KEY-----",
                "private-key",
                Redaction::WholeMatch,
            ),
            (
                r"\b(?:sk-ant-|sk-proj-|sk-)[A-Za-z0-9_\-]{16,}\b",
                "api-key",
                Redaction::WholeMatch,
            ),
            (
                r"\b(?:ghp_|gho_|ghu_|ghs_|ghr_|github_pat_|glpat-)[A-Za-z0-9_\-]{16,}\b",
                "access-token",
                Redaction::WholeMatch,
            ),
            (
                r"\bxox[bpars]-[A-Za-z0-9\-]{10,}\b",
                "access-token",
                Redaction::WholeMatch,
            ),
            (r"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b", "aws-key-id", Redaction::WholeMatch),
            // Real Google API keys are `AIza` plus 35 characters; the floor is
            // set below that so a shortened future shape is still caught.
            (r"\bAIza[A-Za-z0-9_\-]{30,}\b", "api-key", Redaction::WholeMatch),
            // A JWT is three base64url segments; requiring all three and a
            // generous minimum length keeps ordinary base64 out of the net.
            (
                r"\beyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\b",
                "jwt",
                Redaction::WholeMatch,
            ),
            (r"(?i)\b(bearer\s+)[A-Za-z0-9._\-]{20,}", "bearer-token", Redaction::KeepFirstGroup),
            (
                // `name = value` / `name: value`, quoted or not. A name alone is
                // weak evidence -- `let token = fetch_token();` is ordinary code
                // -- so the value is only removed when it also contains a digit,
                // which real generated secrets have and most identifiers do not.
                r#"(?i)\b(api[_-]?key|apikey|access[_-]?token|auth[_-]?token|refresh[_-]?token|client[_-]?secret|private[_-]?key|password|passwd|secret|token)\b(\s*[:=]\s*['"]?)([A-Za-z0-9._\-/+]{12,})"#,
                "assigned-credential",
                Redaction::KeepPrefixIfValueLooksGenerated,
            ),
        ];
        sources
            .iter()
            .map(|(source, kind, redaction)| CredentialPattern {
                regex: regex::Regex::new(source)
                    .unwrap_or_else(|error| panic!("invalid redaction pattern {source}: {error}")),
                kind,
                redaction: *redaction,
            })
            .collect()
    })
}

fn usage_to_json(usage: TokenUsage) -> JsonValue {
    let mut object = BTreeMap::new();
    object.insert("input_tokens".to_string(), JsonValue::Number(i64::from(usage.input_tokens)));
    object.insert("output_tokens".to_string(), JsonValue::Number(i64::from(usage.output_tokens)));
    object.insert(
        "cache_creation_input_tokens".to_string(),
        JsonValue::Number(i64::from(usage.cache_creation_input_tokens)),
    );
    object.insert(
        "cache_read_input_tokens".to_string(),
        JsonValue::Number(i64::from(usage.cache_read_input_tokens)),
    );
    JsonValue::Object(object)
}

fn usage_from_json(value: &JsonValue) -> Result<TokenUsage, SessionError> {
    let object = value
        .as_object()
        .ok_or_else(|| SessionError::Format("usage must be an object".to_string()))?;
    Ok(TokenUsage {
        input_tokens: required_u32(object, "input_tokens")?,
        output_tokens: required_u32(object, "output_tokens")?,
        cache_creation_input_tokens: required_u32(object, "cache_creation_input_tokens")?,
        cache_read_input_tokens: required_u32(object, "cache_read_input_tokens")?,
    })
}

fn required_string(
    object: &BTreeMap<String, JsonValue>,
    key: &str,
) -> Result<String, SessionError> {
    object
        .get(key)
        .and_then(JsonValue::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| SessionError::Format(format!("missing {key}")))
}

fn required_u32(object: &BTreeMap<String, JsonValue>, key: &str) -> Result<u32, SessionError> {
    let value = object
        .get(key)
        .and_then(JsonValue::as_i64)
        .ok_or_else(|| SessionError::Format(format!("missing {key}")))?;
    u32::try_from(value).map_err(|_| SessionError::Format(format!("{key} out of range")))
}

fn required_u64(object: &BTreeMap<String, JsonValue>, key: &str) -> Result<u64, SessionError> {
    let value = object.get(key).ok_or_else(|| SessionError::Format(format!("missing {key}")))?;
    required_u64_from_value(value, key)
}

fn required_u64_from_value(value: &JsonValue, key: &str) -> Result<u64, SessionError> {
    let value = value.as_i64().ok_or_else(|| SessionError::Format(format!("missing {key}")))?;
    u64::try_from(value).map_err(|_| SessionError::Format(format!("{key} out of range")))
}

fn required_usize(object: &BTreeMap<String, JsonValue>, key: &str) -> Result<usize, SessionError> {
    let value = object
        .get(key)
        .and_then(JsonValue::as_i64)
        .ok_or_else(|| SessionError::Format(format!("missing {key}")))?;
    usize::try_from(value).map_err(|_| SessionError::Format(format!("{key} out of range")))
}

fn i64_from_u64(value: u64, key: &str) -> Result<i64, SessionError> {
    i64::try_from(value)
        .map_err(|_| SessionError::Format(format!("{key} out of range for JSON number")))
}

fn i64_from_usize(value: usize, key: &str) -> Result<i64, SessionError> {
    i64::try_from(value)
        .map_err(|_| SessionError::Format(format!("{key} out of range for JSON number")))
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn current_time_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

fn generate_session_id() -> String {
    let millis = current_time_millis();
    let counter = SESSION_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("session-{millis}-{counter}")
}

fn write_atomic(path: &Path, contents: &str) -> Result<(), SessionError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp_path = temporary_path_for(path);
    // Written owner-only: the contents are a transcript, and on a shared host
    // the default mode would leave it readable by every other account.
    crate::oauth::write_private_file(&temp_path, contents.as_bytes())?;
    fs::rename(&temp_path, path)?;
    let _ = crate::oauth::restrict_file_permissions(path);
    Ok(())
}

fn temporary_path_for(path: &Path) -> PathBuf {
    let file_name = path.file_name().and_then(|value| value.to_str()).unwrap_or("session");
    path.with_file_name(format!(
        "{file_name}.tmp-{}-{}",
        current_time_millis(),
        SESSION_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
    ))
}

fn rotate_session_file_if_needed(path: &Path) -> Result<(), SessionError> {
    let Ok(metadata) = fs::metadata(path) else {
        return Ok(());
    };
    if metadata.len() < ROTATE_AFTER_BYTES {
        return Ok(());
    }
    let rotated_path = rotated_log_path(path);
    fs::rename(path, rotated_path)?;
    Ok(())
}

fn rotated_log_path(path: &Path) -> PathBuf {
    let stem = path.file_stem().and_then(|value| value.to_str()).unwrap_or("session");
    path.with_file_name(format!("{stem}.rot-{}.jsonl", current_time_millis()))
}

fn cleanup_rotated_logs(path: &Path) -> Result<(), SessionError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    let stem = path.file_stem().and_then(|value| value.to_str()).unwrap_or("session");
    let prefix = format!("{stem}.rot-");
    let mut rotated_paths = fs::read_dir(parent)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|entry_path| {
            entry_path.file_name().and_then(|value| value.to_str()).is_some_and(|name| {
                name.starts_with(&prefix)
                    && Path::new(name)
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"))
            })
        })
        .collect::<Vec<_>>();

    rotated_paths.sort_by_key(|entry_path| {
        fs::metadata(entry_path).and_then(|metadata| metadata.modified()).unwrap_or(UNIX_EPOCH)
    });

    let remove_count = rotated_paths.len().saturating_sub(MAX_ROTATED_FILES);
    for stale_path in rotated_paths.into_iter().take(remove_count) {
        fs::remove_file(stale_path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        cleanup_rotated_logs, redact_credentials, rotate_session_file_if_needed, ContentBlock,
        ConversationMessage, MessageRole, Session, SessionFork,
    };
    use crate::json::JsonValue;
    use crate::usage::TokenUsage;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn redacts_credentials_that_have_a_recognisable_shape() {
        // The fixtures carry the *shape* of each credential, not a copy of a
        // real one. They are spelled with a `TESTVECTOR` marker on purpose:
        // GitHub's push protection blocks the push when a commit contains a
        // credential-shaped string, and the first version of this table - using
        // plausible-looking filler - had exactly that effect. Rewriting them to
        // be unmistakably fake keeps the shape rule under test without making
        // the repository look like it ships a leaked token.
        let cases = [
            ("sk-ant-api03-TESTVECTORNOTASECRET000000", "Anthropic key"),
            ("sk-proj-TESTVECTORNOTASECRET000000", "OpenAI project key"),
            ("ghp_TESTVECTORNOTASECRET0000000000", "GitHub token"),
            ("github_pat_TESTVECTORNOTASECRET", "fine-grained GitHub token"),
            ("glpat-TESTVECTORNOTASECRET", "GitLab token"),
            ("xoxb-TESTVECTOR-NOT-A-REAL-TOKEN", "Slack token"),
            ("AKIA000000000000TEST", "AWS access key id"),
            ("AIzaTESTVECTORNOTASECRET000000000000", "Google API key"),
            ("eyJ0ZXN0Ijp0cnVlfQ.eyJ0ZXN0Ijp0cnVlfQ.eyJ0ZXN0Ijp0cnVlfQ", "JWT"),
            ("Bearer TESTVECTORNOTASECRET00000000", "bearer token"),
        ];
        for (secret, label) in cases {
            let redacted = redact_credentials(&format!("value is {secret} here"));
            assert!(!redacted.contains(secret), "{label} must not survive redaction: {redacted}");
            assert!(
                redacted.contains("[redacted:"),
                "{label} must leave a marker so a reader can see something was removed: {redacted}"
            );
        }
    }

    #[test]
    fn redacts_secrets_assigned_to_a_credential_named_key() {
        for line in [
            "API_KEY=TESTVECTOR0000000000",
            "api-key: \"TESTVECTOR0000000000\"",
            "client_secret = TESTVECTOR0000000000",
            "PASSWORD='TESTVECTOR0000000000'",
        ] {
            let redacted = redact_credentials(line);
            assert!(
                !redacted.contains("TESTVECTOR0000000000"),
                "the value in `{line}` must not survive redaction: {redacted}"
            );
        }
    }

    #[test]
    fn redacts_a_pem_private_key_block_including_its_body() {
        let text = "before\n-----BEGIN RSA PRIVATE KEY-----\nNOT-A-KEY-JUST-A-TEST-VECTOR\n5678\n-----END RSA PRIVATE KEY-----\nafter";
        let redacted = redact_credentials(text);
        assert!(!redacted.contains("NOT-A-KEY-JUST-A-TEST-VECTOR"), "key body must be removed");
        assert!(!redacted.contains("-----END"), "the end marker must be removed too");
        assert!(redacted.starts_with("before"), "surrounding text must be preserved");
        assert!(redacted.ends_with("after"), "surrounding text must be preserved");
    }

    #[test]
    fn leaves_ordinary_code_and_prose_alone() {
        // The false-positive side matters as much as the true-positive side: a
        // filter that mangles ordinary transcripts would make `--resume` replay
        // corrupted context.
        let untouched = [
            "let token = fetch_token();",
            "token = some_function_name",
            "if api_key.is_empty() { return; }",
            "the secret to good review is a small diff",
            "password: required",
            "Authorization: Bearer placeholder",
            "grep -rn \"api_key\" src/",
            "eyJhbGciOiJIUzI1NiJ9",
        ];
        for text in untouched {
            assert_eq!(redact_credentials(text), text, "`{text}` must be left unchanged");
        }
    }

    #[test]
    fn persisted_transcripts_redact_messages_but_keep_the_record_readable() {
        let mut session = Session::new();
        session
            .push_user_text("deploy with sk-ant-api03-TESTVECTORNOTASECRET000000 please")
            .expect("user message should append");
        session
            .push_message(ConversationMessage::tool_result(
                "tool-1",
                "bash",
                "export GH_TOKEN=ghp_TESTVECTORNOTASECRET0000000000",
                false,
            ))
            .expect("tool result should append");

        let path = temp_session_path("redaction");
        session.save_to_path(&path).expect("session should save");
        let contents = fs::read_to_string(&path).expect("session file should be readable");
        let restored = Session::load_from_path(&path).expect("redacted session must still parse");
        fs::remove_file(&path).expect("temp file should be removable");

        assert!(!contents.contains("sk-ant-api03"), "the key must not reach disk:\n{contents}");
        assert!(!contents.contains("ghp_TESTVECTOR"), "the token must not reach disk:\n{contents}");
        // Redaction must not have broken the line format: the transcript is
        // still one JSON object per line, and still replays.
        for line in contents.lines() {
            assert!(!line.is_empty());
            assert!(
                JsonValue::parse(line).is_ok(),
                "a redacted record must remain valid JSON: {line}"
            );
        }
        assert_eq!(restored.messages.len(), 2);
        // The original session is untouched, so the live turn still sees what
        // the user actually typed.
        assert!(session.messages[0].blocks.iter().any(
            |block| matches!(block, ContentBlock::Text { text } if text.contains("sk-ant-api03"))
        ));
    }

    #[test]
    #[cfg(unix)]
    fn a_saved_transcript_is_not_readable_by_other_users() {
        use std::os::unix::fs::PermissionsExt;

        let mut session = Session::new();
        session.push_user_text("hello").expect("user message should append");
        let path = temp_session_path("private");
        session.save_to_path(&path).expect("session should save");

        let mode = fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
        fs::remove_file(&path).expect("temp file should be removable");
        assert_eq!(mode, 0o600, "a transcript is user state, not shared state");
    }

    #[test]
    fn redaction_is_idempotent() {
        // A resumed transcript is redacted again when it is saved again. If the
        // second pass changed anything, markers would accumulate on every save.
        let text =
            "key=sk-ant-api03-TESTVECTORNOTASECRET000000 and Bearer TESTVECTORNOTASECRET00000000";
        let once = redact_credentials(text);
        let twice = redact_credentials(&once);
        assert_eq!(once, twice, "a second pass must be a no-op");
        assert!(once.contains("[redacted:"), "the first pass must redact something");
    }

    #[test]
    fn appended_messages_are_redacted_too() {
        // The snapshot path and the append path are separate writers; a fix that
        // only covered `save_to_path` would leak every message after the first.
        let path = temp_session_path("append-redaction");
        let mut session = Session::new().with_persistence_path(&path);
        session.push_user_text("bootstrap").expect("first message should persist");
        session
            .push_user_text("my key is ghp_TESTVECTORNOTASECRET0000000000")
            .expect("second message should append");

        let contents = fs::read_to_string(&path).expect("session file should be readable");
        fs::remove_file(&path).expect("temp file should be removable");
        assert!(
            !contents.contains("ghp_TESTVECTOR"),
            "the appended line must be redacted as well:\n{contents}"
        );
    }

    #[test]
    fn persists_and_restores_session_jsonl() {
        let mut session = Session::new();
        session.push_user_text("hello").expect("user message should append");
        session
            .push_message(ConversationMessage::assistant_with_usage(
                vec![
                    ContentBlock::Text { text: "thinking".to_string() },
                    ContentBlock::ToolUse {
                        id: "tool-1".to_string(),
                        name: "bash".to_string(),
                        input: "echo hi".to_string(),
                    },
                ],
                Some(TokenUsage {
                    input_tokens: 10,
                    output_tokens: 4,
                    cache_creation_input_tokens: 1,
                    cache_read_input_tokens: 2,
                }),
            ))
            .expect("assistant message should append");
        session
            .push_message(ConversationMessage::tool_result("tool-1", "bash", "hi", false))
            .expect("tool result should append");

        let path = temp_session_path("jsonl");
        session.save_to_path(&path).expect("session should save");
        let restored = Session::load_from_path(&path).expect("session should load");
        fs::remove_file(&path).expect("temp file should be removable");

        assert_eq!(restored, session);
        assert_eq!(restored.messages[2].role, MessageRole::Tool);
        assert_eq!(restored.messages[1].usage.expect("usage").total_tokens(), 17);
        assert_eq!(restored.session_id, session.session_id);
    }

    #[test]
    fn loads_legacy_session_json_object() {
        let path = temp_session_path("legacy");
        let legacy = JsonValue::Object(
            [
                ("version".to_string(), JsonValue::Number(1)),
                (
                    "messages".to_string(),
                    JsonValue::Array(vec![ConversationMessage::user_text("legacy").to_json()]),
                ),
            ]
            .into_iter()
            .collect(),
        );
        fs::write(&path, legacy.render()).expect("legacy file should write");

        let restored = Session::load_from_path(&path).expect("legacy session should load");
        fs::remove_file(&path).expect("temp file should be removable");

        assert_eq!(restored.messages.len(), 1);
        assert_eq!(restored.messages[0], ConversationMessage::user_text("legacy"));
        assert!(!restored.session_id.is_empty());
    }

    #[test]
    fn appends_messages_to_persisted_jsonl_session() {
        let path = temp_session_path("append");
        let mut session = Session::new().with_persistence_path(path.clone());
        session.save_to_path(&path).expect("initial save should succeed");
        session.push_user_text("hi").expect("user append should succeed");
        session
            .push_message(ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "hello".to_string(),
            }]))
            .expect("assistant append should succeed");

        let restored = Session::load_from_path(&path).expect("session should replay from jsonl");
        fs::remove_file(&path).expect("temp file should be removable");

        assert_eq!(restored.messages.len(), 2);
        assert_eq!(restored.messages[0], ConversationMessage::user_text("hi"));
    }

    #[test]
    fn persists_compaction_metadata() {
        let path = temp_session_path("compaction");
        let mut session = Session::new();
        session.push_user_text("before").expect("message should append");
        session.record_compaction("summarized earlier work", 4);
        session.save_to_path(&path).expect("session should save");

        let restored = Session::load_from_path(&path).expect("session should load");
        fs::remove_file(&path).expect("temp file should be removable");

        let compaction = restored.compaction.expect("compaction metadata");
        assert_eq!(compaction.count, 1);
        assert_eq!(compaction.removed_message_count, 4);
        assert!(compaction.summary.contains("summarized"));
    }

    #[test]
    fn forks_sessions_with_branch_metadata_and_persists_it() {
        let path = temp_session_path("fork");
        let mut session = Session::new();
        session.push_user_text("before fork").expect("message should append");

        let forked =
            session.fork(Some("investigation".to_string())).with_persistence_path(path.clone());
        forked.save_to_path(&path).expect("forked session should save");

        let restored = Session::load_from_path(&path).expect("forked session should load");
        fs::remove_file(&path).expect("temp file should be removable");

        assert_ne!(restored.session_id, session.session_id);
        assert_eq!(
            restored.fork,
            Some(SessionFork {
                parent_session_id: session.session_id,
                branch_name: Some("investigation".to_string()),
            })
        );
        assert_eq!(restored.messages, forked.messages);
    }

    #[test]
    fn rotates_and_cleans_up_large_session_logs() {
        // given
        let path = temp_session_path("rotation");
        let oversized_length =
            usize::try_from(super::ROTATE_AFTER_BYTES + 10).expect("rotate threshold should fit");
        fs::write(&path, "x".repeat(oversized_length)).expect("oversized file should write");

        // when
        rotate_session_file_if_needed(&path).expect("rotation should succeed");

        // then
        assert!(!path.exists(), "original path should be rotated away before rewrite");

        for _ in 0..5 {
            let rotated = super::rotated_log_path(&path);
            fs::write(&rotated, "old").expect("rotated file should write");
        }
        cleanup_rotated_logs(&path).expect("cleanup should succeed");

        let rotated_count = rotation_files(&path).len();
        assert!(rotated_count <= super::MAX_ROTATED_FILES);
        for rotated in rotation_files(&path) {
            fs::remove_file(rotated).expect("rotated file should be removable");
        }
    }

    #[test]
    fn rejects_jsonl_record_without_type() {
        // given
        let path = write_temp_session_file(
            "missing-type",
            r#"{"message":{"role":"user","blocks":[{"type":"text","text":"hello"}]}}"#,
        );

        // when
        let error = Session::load_from_path(&path)
            .expect_err("session should reject JSONL records without a type");

        // then
        assert!(error.to_string().contains("missing type"));
        fs::remove_file(path).expect("temp file should be removable");
    }

    #[test]
    fn rejects_jsonl_message_record_without_message_payload() {
        // given
        let path = write_temp_session_file("missing-message", r#"{"type":"message"}"#);

        // when
        let error = Session::load_from_path(&path)
            .expect_err("session should reject JSONL message records without message payload");

        // then
        assert!(error.to_string().contains("missing message"));
        fs::remove_file(path).expect("temp file should be removable");
    }

    #[test]
    fn rejects_jsonl_record_with_unknown_type() {
        // given
        let path = write_temp_session_file("unknown-type", r#"{"type":"mystery"}"#);

        // when
        let error = Session::load_from_path(&path)
            .expect_err("session should reject unknown JSONL record types");

        // then
        assert!(error.to_string().contains("unsupported JSONL record type"));
        fs::remove_file(path).expect("temp file should be removable");
    }

    #[test]
    fn rejects_legacy_session_json_without_messages() {
        // given
        let session = JsonValue::Object(
            [("version".to_string(), JsonValue::Number(1))].into_iter().collect(),
        );

        // when
        let error = Session::from_json(&session)
            .expect_err("legacy session objects should require messages");

        // then
        assert!(error.to_string().contains("missing messages"));
    }

    #[test]
    fn normalizes_blank_fork_branch_name_to_none() {
        // given
        let session = Session::new();

        // when
        let forked = session.fork(Some("   ".to_string()));

        // then
        assert_eq!(forked.fork.expect("fork metadata").branch_name, None);
    }

    #[test]
    fn rejects_unknown_content_block_type() {
        // given
        let block = JsonValue::Object(
            [("type".to_string(), JsonValue::String("unknown".to_string()))].into_iter().collect(),
        );

        // when
        let error = ContentBlock::from_json(&block)
            .expect_err("content blocks should reject unknown types");

        // then
        assert!(error.to_string().contains("unsupported block type"));
    }

    fn temp_session_path(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("runtime-session-{label}-{nanos}.json"))
    }

    fn write_temp_session_file(label: &str, contents: &str) -> PathBuf {
        let path = temp_session_path(label);
        fs::write(&path, format!("{contents}\n")).expect("temp session file should write");
        path
    }

    fn rotation_files(path: &Path) -> Vec<PathBuf> {
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .expect("temp path should have file stem")
            .to_string();
        fs::read_dir(path.parent().expect("temp path should have parent"))
            .expect("temp dir should read")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|entry_path| {
                entry_path.file_name().and_then(|value| value.to_str()).is_some_and(|name| {
                    name.starts_with(&format!("{stem}.rot-"))
                        && Path::new(name)
                            .extension()
                            .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"))
                })
            })
            .collect()
    }
}
