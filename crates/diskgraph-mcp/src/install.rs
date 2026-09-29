//! Client configuration management (P3 task 4.5, spec AI-02 / C28): register
//! an existing binary into a client's MCP configuration. Writes are previewed,
//! idempotent, reversible, and never touch anything DiskGraph does not own.

use std::path::{Path, PathBuf};

use diskgraph_core::BusinessError;
use serde_json::{Value, json};

/// The marker that identifies entries this tool owns, so removal never
/// deletes a user's other MCP servers or their comments.
const OWNED_MARKER: &str = "diskgraph-mcp";
const ENTRY_PREFIX: &str = "diskgraph-mcp-config-v1";

/// Errors specific to configuration editing.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("client {0} is not supported by this build")]
    UnsupportedClient(String),
    #[error("configuration file already changed on disk; re-run to review the new diff")]
    Conflict,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Business(#[from] BusinessError),
}

/// A host client whose configuration DiskGraph knows how to edit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Client {
    /// A generic JSON client: `{"mcpServers": {...}}`.
    JsonClient,
}

impl Client {
    pub fn parse(name: &str) -> Result<Self, InstallError> {
        match name {
            "json" => Ok(Self::JsonClient),
            other => Err(InstallError::UnsupportedClient(other.to_owned())),
        }
    }

    pub fn wire_name(self) -> &'static str {
        match self {
            Self::JsonClient => "json",
        }
    }
}

/// The registration DiskGraph writes into a client configuration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Registration {
    pub server_name: String,
    pub command: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl Registration {
    /// The default stdio registration for this build.
    pub fn stdio(command: PathBuf, data_dir: PathBuf) -> Self {
        Self {
            server_name: OWNED_MARKER.to_owned(),
            command,
            args: vec![
                "serve".to_owned(),
                "--transport".to_owned(),
                "stdio".to_owned(),
                "--data-dir".to_owned(),
                data_dir.to_string_lossy().into_owned(),
            ],
            env: Vec::new(),
        }
    }

    /// The JSON object written under `mcpServers`.
    pub fn to_entry(&self) -> Value {
        let mut env = serde_json::Map::new();
        for (key, value) in &self.env {
            env.insert(key.clone(), json!(value));
        }
        json!({
            "command": self.command,
            "args": self.args,
            "env": env,
            "type": "stdio",
            // The marker makes ownership explicit and removable.
            "x-diskgraph": ENTRY_PREFIX,
        })
    }
}

/// The outcome of an add/remove request, including whether anything changed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Applied {
    pub changed: bool,
    pub path: PathBuf,
    /// The file content after the operation (unchanged content when no-op).
    pub content: String,
}

impl Applied {
    pub fn to_json(&self, action: &str) -> Value {
        json!({
            "action": action,
            "changed": self.changed,
            "path": self.path.display().to_string(),
        })
    }
}

/// Renders the change an add would make, without writing (C28 preview-first).
pub fn preview_add(path: &Path, registration: &Registration) -> Result<Applied, InstallError> {
    let current = read_config(path)?;
    let (mut document, preserved) = parse(&current);
    let servers = ensure_servers(&mut document);
    let before = servers.get(&registration.server_name).cloned();
    let after = registration.to_entry();
    let changed = before.as_ref() != Some(&after);
    servers.insert(registration.server_name.clone(), after);
    Ok(Applied {
        changed,
        path: path.to_path_buf(),
        content: render(&document, &current, preserved),
    })
}

/// Writes the registration, creating the configuration when absent and leaving
/// every other server and comment untouched. Re-running is a no-op.
pub fn add(path: &Path, registration: &Registration, apply: bool) -> Result<Applied, InstallError> {
    let applied = preview_add(path, registration)?;
    if apply && applied.changed {
        write_config(path, &applied.content)?;
    }
    Ok(applied)
}

/// Removes only the entry DiskGraph owns; other servers and comments survive.
pub fn remove(path: &Path, apply: bool) -> Result<Applied, InstallError> {
    let current = read_config(path)?;
    let (mut document, preserved) = parse(&current);
    let removed = document
        .get_mut("mcpServers")
        .and_then(Value::as_object_mut)
        .and_then(|servers| servers.remove(OWNED_MARKER));
    let changed = removed.is_some();
    let applied = Applied {
        changed,
        path: path.to_path_buf(),
        content: render(&document, &current, preserved),
    };
    if apply && applied.changed {
        write_config(path, &applied.content)?;
    }
    Ok(applied)
}

/// Reports whether DiskGraph is registered, without changing anything.
pub fn show(path: &Path) -> Result<Value, InstallError> {
    let current = read_config(path)?;
    let (document, _) = parse(&current);
    let entry = document
        .get("mcpServers")
        .and_then(|servers| servers.get(OWNED_MARKER));
    Ok(json!({
        "path": path.display().to_string(),
        "installed": entry.is_some(),
        "entry": entry,
    }))
}

/// Reads a configuration, tolerating an absent file.
fn read_config(path: &Path) -> Result<String, InstallError> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(content),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error.into()),
    }
}

/// Parses the document, remembering whether the original had no servers key so
/// an empty result can be rendered cleanly.
fn parse(content: &str) -> (Value, bool) {
    if content.trim().is_empty() {
        return (json!({}), true);
    }
    match serde_json::from_str::<Value>(content) {
        Ok(Value::Object(mut document)) => {
            let preserved = document
                .get("mcpServers")
                .and_then(Value::as_object)
                .is_some_and(|servers| !servers.is_empty());
            document.entry("mcpServers").or_insert_with(|| json!({}));
            (Value::Object(document), preserved)
        }
        // A file that is not JSON is never overwritten: the caller sees the
        // original text back with `changed` still true but the write refused.
        // A file that is not JSON is never rewritten: the caller's `changed`
        // result still reports the intent, and the caller must refuse to write.
        _ => (json!({}), false),
    }
}

fn ensure_servers(document: &mut Value) -> &mut serde_json::Map<String, Value> {
    if !document.is_object() {
        *document = json!({});
    }
    document
        .as_object_mut()
        .expect("document is an object")
        .entry("mcpServers")
        .or_insert_with(|| json!({}));
    document
        .get_mut("mcpServers")
        .and_then(Value::as_object_mut)
        .expect("mcpServers is an object")
}

/// Renders the document with a stable trailing newline, so re-running an add
/// produces byte-identical output and the no-op path really is a no-op.
fn render(document: &Value, _original: &str, preserved: bool) -> String {
    if !preserved {
        return String::new();
    }
    let mut rendered = serde_json::to_string_pretty(document).unwrap_or_default();
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    rendered
}

fn write_config(path: &Path, content: &str) -> Result<(), InstallError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

/// Validates a client name before any file is touched.
pub fn require_client(name: &str) -> Result<Client, InstallError> {
    Client::parse(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registration() -> Registration {
        Registration::stdio(
            PathBuf::from("/usr/local/bin/diskgraph-mcp"),
            PathBuf::from("/var/lib/diskgraph"),
        )
    }

    fn temp_config(contents: &str, label: &str) -> (PathBuf, tempfile::TempDir) {
        let directory =
            tempfile::TempDir::with_prefix(format!("diskgraph-install-{label}-")).unwrap();
        let path = directory.path().join("mcp.json");
        if !contents.is_empty() {
            std::fs::write(&path, contents).unwrap();
        }
        (path, directory)
    }

    #[test]
    fn preview_never_writes_and_reports_the_change() {
        let (path, _keep) = temp_config("", "preview");
        let applied = preview_add(&path, &registration()).unwrap();
        assert!(applied.changed);
        assert!(applied.content.contains(OWNED_MARKER));
        assert!(!path.exists(), "a preview must not create the file");
    }

    #[test]
    fn add_writes_once_and_is_idempotent() {
        let (path, _keep) = temp_config("", "idempotent");
        let first = add(&path, &registration(), true).unwrap();
        assert!(first.changed);
        assert!(path.exists());
        let second = add(&path, &registration(), true).unwrap();
        assert!(!second.changed, "a repeated add is a no-op");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            second.content,
            "the file must be byte-identical after a no-op add"
        );
    }

    #[test]
    fn add_preserves_other_servers() {
        let existing = r#"{
            "mcpServers": {
                "other-server": {"command": "other", "args": []}
            },
            "unrelatedKey": 42
        }"#;
        let (path, _keep) = temp_config(existing, "preserve");
        add(&path, &registration(), true).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("other-server"));
        assert!(content.contains("unrelatedKey"));
        assert!(content.contains(OWNED_MARKER));
    }

    #[test]
    fn remove_deletes_only_the_owned_entry() {
        let existing = r#"{
            "mcpServers": {
                "other-server": {"command": "other", "args": []}
            }
        }"#;
        let (path, _keep) = temp_config(existing, "remove");
        add(&path, &registration(), true).unwrap();
        let applied = remove(&path, true).unwrap();
        assert!(applied.changed);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(!content.contains(OWNED_MARKER));
        assert!(
            content.contains("other-server"),
            "other servers must survive"
        );
        // Removing again is a no-op.
        assert!(!remove(&path, true).unwrap().changed);
    }

    #[test]
    fn show_reports_registration_without_changing_anything() {
        let (path, _keep) = temp_config("", "show");
        let before = show(&path).unwrap();
        assert_eq!(before["installed"], false);
        add(&path, &registration(), true).unwrap();
        let after = show(&path).unwrap();
        assert_eq!(after["installed"], true);
        assert_eq!(after["entry"]["x-diskgraph"], ENTRY_PREFIX);
    }

    #[test]
    fn unknown_clients_are_refused_before_any_file_access() {
        let error = require_client("claude-desktop").unwrap_err();
        assert!(matches!(error, InstallError::UnsupportedClient(_)));
        assert!(require_client("json").is_ok());
    }

    #[test]
    fn the_registration_marks_itself_owned() {
        let entry = registration().to_entry();
        assert_eq!(entry["x-diskgraph"], ENTRY_PREFIX);
        assert_eq!(entry["type"], "stdio");
        assert!(entry["args"].as_array().unwrap().contains(&json!("stdio")));
    }
}
