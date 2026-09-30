//! Agent instruction files: the part of `diskgraph init` that makes an index
//! reachable.
//!
//! An index nothing points at is a directory of numbers. Each supported
//! agent reads its instructions from one file, so a project gets a marked
//! block written into that file and the agent knows the index exists.
//!
//! Two rules govern every write here. The block is bracketed by markers, so
//! re-running replaces our own text and nothing else - a file the user has
//! been editing for a year comes back with its every other line intact.
//! And a block that already says what we would write is not written again,
//! so `init` leaves no trace on disk when there is nothing to change.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;

const START: &str = "<!-- DISKGRAPH_START -->";
const END: &str = "<!-- DISKGRAPH_END -->";

/// One agent platform DiskGraph can write instructions for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Target {
    Claude,
    Codex,
    Kimi,
}

impl Target {
    pub const ALL: [Target; 3] = [Target::Claude, Target::Codex, Target::Kimi];

    pub fn name(self) -> &'static str {
        match self {
            Target::Claude => "claude",
            Target::Codex => "codex",
            Target::Kimi => "kimi",
        }
    }

    /// Resolves a `--target` value, which may name several at once.
    pub fn parse_list(value: &str) -> Result<Vec<Target>, String> {
        let mut out = Vec::new();
        for part in value
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            match part.to_ascii_lowercase().as_str() {
                "claude" => out.push(Target::Claude),
                "codex" => out.push(Target::Codex),
                "kimi" => out.push(Target::Kimi),
                other => return Err(format!("unknown target: {other}")),
            }
        }
        if out.is_empty() {
            return Err("no target given".into());
        }
        out.dedup();
        Ok(out)
    }

    /// The file this agent reads user-level instructions from.
    pub fn global_file(self) -> Option<PathBuf> {
        let home = home_dir()?;
        Some(match self {
            // Honour the env overrides both agents document, so a user who
            // moved their config did not get it moved back by us.
            Target::Claude => std::env::var_os("CLAUDE_CONFIG_DIR").map_or_else(
                || home.join(".claude").join("CLAUDE.md"),
                |dir| PathBuf::from(dir).join("CLAUDE.md"),
            ),
            Target::Codex => std::env::var_os("CODEX_HOME").map_or_else(
                || home.join(".codex").join("AGENTS.md"),
                |dir| PathBuf::from(dir).join("AGENTS.md"),
            ),
            // Kimi keeps its configuration in one JSON file rather than a
            // markdown one, so the project-level file is the honest target.
            Target::Kimi => return None,
        })
    }

    /// The file this agent reads project-level instructions from.
    pub fn project_file(self, project: &Path) -> PathBuf {
        match self {
            Target::Claude => project.join(".claude").join("CLAUDE.md"),
            // Codex reads AGENTS.md from the repository root, not from a
            // .codex subdirectory: writing it elsewhere would be silent.
            Target::Codex => project.join("AGENTS.md"),
            Target::Kimi => project.join("KIMI.md"),
        }
    }

    /// Whether this agent looks like it is set up on this machine.
    pub fn detected(self) -> bool {
        let home = match home_dir() {
            Some(home) => home,
            None => return false,
        };
        match self {
            Target::Claude => home.join(".claude").is_dir(),
            Target::Codex => home.join(".codex").is_dir(),
            Target::Kimi => home.join(".kimi").is_dir(),
        }
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_dir())
}

/// What happened to one file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    /// The file did not exist and now holds only the block.
    Created,
    /// The block was written over a previous one.
    Updated,
    /// The block was already there and says the same thing.
    Unchanged,
    /// The block was removed.
    Removed,
    /// Nothing to do, and nothing done: a missing global home is not an error.
    Skipped,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Created => "created",
            Outcome::Updated => "updated",
            Outcome::Unchanged => "unchanged",
            Outcome::Removed => "removed",
            Outcome::Skipped => "skipped",
        }
    }

    /// Whether this outcome touched the disk. A second `init` that finds the
    /// block already says this reports false, and nothing was written.
    pub fn wrote(self) -> bool {
        matches!(self, Outcome::Created | Outcome::Updated | Outcome::Removed)
    }
}

/// Writes the block into `path`, leaving every other byte alone.
///
/// Returns what it did without writing anything when the file already says
/// exactly this: a second `init` must be silent on disk, or every agent would
/// see a modified timestamp for a file whose content never changed.
pub fn upsert_block(path: &Path, block: &str) -> std::io::Result<Outcome> {
    let existing = read_to_string(path)?;
    let Some(existing) = existing else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_atomically(path, &(render_block(block) + "\n"))?;
        return Ok(Outcome::Created);
    };
    let Some(before) = existing.split(START).next() else {
        // No marker: append, keeping the user's file exactly as it was.
        let separator = if existing.trim().is_empty() {
            ""
        } else {
            "\n\n"
        };
        let updated = format!("{existing}{separator}{}\n", render_block(block));
        if updated == existing {
            return Ok(Outcome::Unchanged);
        }
        write_atomically(path, &updated)?;
        return Ok(Outcome::Updated);
    };
    let after = existing
        .split_once(END)
        .map(|(_, tail)| tail)
        .unwrap_or_default();
    // The tail keeps the whitespace that followed our marker, so the user's
    // content after the block is preserved byte for byte rather than
    // re-indented - and a file we created ourselves compares equal to
    // itself, which is what makes a second init write nothing.
    let updated = format!("{before}{}{after}", render_block(block));
    if updated == existing {
        return Ok(Outcome::Unchanged);
    }
    write_atomically(path, &updated)?;
    Ok(Outcome::Updated)
}

/// Removes the block, and reports whether there was one. A file that becomes
/// empty is removed too, so an uninstall does not leave a stub behind.
pub fn remove_block(path: &Path) -> std::io::Result<Outcome> {
    let Some(existing) = read_to_string(path)? else {
        return Ok(Outcome::Skipped);
    };
    if !existing.contains(START) {
        return Ok(Outcome::Skipped);
    }
    let before = existing.split(START).next().unwrap_or_default();
    let after = existing
        .split_once(END)
        .map(|(_, tail)| tail)
        .unwrap_or_default();
    let remainder = format!("{before}{after}");
    if remainder.trim().is_empty() {
        std::fs::remove_file(path)?;
        return Ok(Outcome::Removed);
    }
    write_atomically(path, &(remainder.trim_end().to_owned() + "\n"))?;
    Ok(Outcome::Removed)
}

fn render_block(body: &str) -> String {
    format!("{START}\n{body}\n{END}")
}

fn read_to_string(path: &Path) -> std::io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Writes through a temporary file in the same directory and renames it over
/// the target, so a reader never sees a half-written instruction file and a
/// crash mid-write leaves the previous one intact.
fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.diskgraph-tmp",
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "instructions".into())
    ));
    {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
    }
    std::fs::rename(&temporary, path)
}

/// The block body, in the language the agent's own instructions are in.
///
/// Conditional on purpose. A file under `~` applies to every project on the
/// machine, including the ones with no index, and an unconditional "use
/// diskgraph" there sends agents looking for a tool in directories that have
/// none. The exit condition is stated rather than implied.
pub fn instruction_body(locale: &str) -> &'static str {
    match locale {
        "zh" => include_str!("assets/instructions-zh.md"),
        _ => include_str!("assets/instructions-en.md"),
    }
}

/// Resolves the locale from the environment, defaulting to English.
pub fn locale_from_env() -> String {
    for name in ["DISKGRAPH_LOCALE", "LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(value) = std::env::var_os(name) {
            let value = value.to_string_lossy().to_ascii_lowercase();
            if value.starts_with("zh") {
                return "zh".into();
            }
        }
    }
    "en".into()
}

/// One file to write, and the target it belongs to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstructionFile {
    pub target: Target,
    pub path: PathBuf,
    pub global: bool,
}

/// Every file the given targets want written, in a stable order.
pub fn instruction_files(targets: &[Target], project: &Path, global: bool) -> Vec<InstructionFile> {
    targets
        .iter()
        .filter_map(|target| {
            let path = if global {
                target.global_file()?
            } else {
                target.project_file(project)
            };
            Some(InstructionFile {
                target: *target,
                path,
                global,
            })
        })
        .collect()
}

/// Reads the agent's own config, if it has one worth mentioning. Kimi's
/// `mcp.json` is the one diskgraph does not have a global markdown file for,
/// so the report says where its MCP entry belongs instead of writing a file
/// no reader will open.
pub fn kimi_mcp_config_path() -> Option<PathBuf> {
    let home = home_dir()?;
    let path = home.join(".kimi").join("mcp.json");
    path.is_file().then_some(path)
}

/// Whether a Kimi config already mentions diskgraph, for the report.
pub fn kimi_has_diskgraph(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .is_some_and(|config| text_mentions(&config, "diskgraph"))
}

fn text_mentions(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(text) => text.to_ascii_lowercase().contains(needle),
        Value::Array(items) => items.iter().any(|item| text_mentions(item, needle)),
        Value::Object(fields) => fields.values().any(|field| text_mentions(field, needle)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::TempDir::with_prefix(label).unwrap();
        let path = dir.path().join("AGENTS.md");
        (dir, path)
    }

    const BLOCK: &str = "## DiskGraph\n\nUse it.";

    #[test]
    fn a_missing_file_is_created_holding_the_block() {
        let (_dir, path) = scratch("dg-inst-create-");
        assert_eq!(upsert_block(&path, BLOCK).unwrap(), Outcome::Created);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(START));
        assert!(text.trim_end().ends_with(END));
    }

    #[test]
    fn a_file_without_a_marker_gains_the_block_with_its_content_intact() {
        let (_dir, path) = scratch("dg-inst-append-");
        std::fs::write(&path, "# My notes\n\nAlways respond in pirate.\n").unwrap();
        assert_eq!(upsert_block(&path, BLOCK).unwrap(), Outcome::Updated);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("Always respond in pirate."),
            "the user's own line must survive: {text}"
        );
        assert!(text.contains(BLOCK));
    }

    #[test]
    fn a_second_identical_write_touches_nothing() {
        let (_dir, path) = scratch("dg-inst-stable-");
        upsert_block(&path, BLOCK).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        assert_eq!(upsert_block(&path, BLOCK).unwrap(), Outcome::Unchanged);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), first);
    }

    #[test]
    fn rewriting_replaces_only_our_block() {
        let (_dir, path) = scratch("dg-inst-replace-");
        std::fs::write(
            &path,
            format!(
                "# Header\n\nkeep me\n\n{}\n\nkeep me too\n",
                render_block("old")
            ),
        )
        .unwrap();
        assert_eq!(upsert_block(&path, BLOCK).unwrap(), Outcome::Updated);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("keep me"));
        assert!(text.contains("keep me too"));
        assert!(text.contains(BLOCK), "our new text is in: {text}");
        assert!(!text.contains("old"), "the previous block is gone: {text}");
        // And it stays stable afterwards, so replacing is idempotent.
        assert_eq!(upsert_block(&path, BLOCK).unwrap(), Outcome::Unchanged);
    }

    #[test]
    fn a_stale_block_is_replaced_rather_than_appended_a_second_time() {
        let (_dir, path) = scratch("dg-inst-stale-");
        upsert_block(&path, "version one").unwrap();
        assert_eq!(
            upsert_block(&path, "version two").unwrap(),
            Outcome::Updated
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches(START).count(), 1, "one block, not two");
        assert!(text.contains("version two"));
    }

    #[test]
    fn removal_takes_the_block_and_leaves_the_rest() {
        let (_dir, path) = scratch("dg-inst-remove-");
        std::fs::write(&path, "# Header\n\nkeep me\n").unwrap();
        upsert_block(&path, BLOCK).unwrap();
        assert_eq!(remove_block(&path).unwrap(), Outcome::Removed);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("keep me"));
        assert!(!text.contains(START));
    }

    #[test]
    fn removing_from_a_file_we_never_wrote_is_a_no_op() {
        let (_dir, path) = scratch("dg-inst-noremove-");
        std::fs::write(&path, "# Their file\n").unwrap();
        let before = std::fs::read_to_string(&path).unwrap();
        assert_eq!(remove_block(&path).unwrap(), Outcome::Skipped);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        // A missing file is skipped too, never an error: an uninstall on a
        // machine that never installed is not a failure.
        let absent = path.with_file_name("never-written.md");
        assert_eq!(remove_block(&absent).unwrap(), Outcome::Skipped);
    }

    #[test]
    fn a_file_that_became_only_our_block_is_removed_entirely() {
        let (_dir, path) = scratch("dg-inst-onlyours-");
        upsert_block(&path, BLOCK).unwrap();
        assert_eq!(remove_block(&path).unwrap(), Outcome::Removed);
        assert!(!path.exists(), "no stub file is left behind");
    }

    #[test]
    fn writes_create_the_directory_they_need() {
        let dir = tempfile::TempDir::with_prefix("dg-inst-mkdir-").unwrap();
        let path = dir.path().join("nested/deeper/AGENTS.md");
        assert_eq!(upsert_block(&path, BLOCK).unwrap(), Outcome::Created);
        assert!(path.is_file());
    }

    #[test]
    fn targets_parse_and_reject_what_they_do_not_know() {
        assert_eq!(
            Target::parse_list("claude,codex").unwrap(),
            vec![Target::Claude, Target::Codex]
        );
        assert_eq!(
            Target::parse_list("CODEX , codex").unwrap(),
            vec![Target::Codex],
            "case and spacing do not matter, and a repeat is not twice"
        );
        assert!(Target::parse_list("cursor").is_err());
        assert!(Target::parse_list("").is_err());
    }

    #[test]
    fn the_shipped_bodies_state_the_exit_condition() {
        for body in [instruction_body("en"), instruction_body("zh")] {
            assert!(
                body.contains(".diskgraph"),
                "the body must say how to tell an indexed directory apart"
            );
            assert!(
                body.contains("diskgraph du") || body.contains("diskgraph top"),
                "the body must name a command the agent can actually run"
            );
        }
    }

    #[test]
    fn neither_body_asks_the_agent_to_change_or_delete_anything() {
        // The whole point of the exit condition is that an agent without an
        // index uses ordinary tools. A body that read as "clean up what is
        // large" would turn a size report into an unasked-for deletion
        // recommendation, so the wording has to stay a report.
        for body in [instruction_body("en"), instruction_body("zh")] {
            let lower = body.to_lowercase();
            for forbidden in ["run diskgraph purge", "diskgraph trash", "diskgraph move"] {
                assert!(
                    !lower.contains(forbidden),
                    "{forbidden} must not appear in the agent instructions"
                );
            }
        }
    }

    #[test]
    fn every_target_has_a_project_file_and_its_name_is_stable() {
        let project = Path::new("/tmp/project");
        for target in Target::ALL {
            let path = target.project_file(project);
            assert!(
                path.starts_with(project),
                "{} must stay inside the project",
                target.name()
            );
            assert!(!target.name().is_empty());
        }
        // Codex reads the repository root, not a dot-directory: this is the
        // one place where picking the obvious-looking path would be silent.
        assert_eq!(
            Target::Codex.project_file(project),
            project.join("AGENTS.md")
        );
    }
}
