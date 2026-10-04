//! DiskGraph CLI: one binary, JSON on stdout, business exit codes on stderr
//! (P2 task 3.12, specs CMD-01 / CMD-02). Commands map onto the shared engine;
//! none of them execute file mutations.

use crate::snapshot_action::SnapshotAction;
mod snapshot_action;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use diskgraph_core::{
    Authorizer, BusinessError, CursorContext, Envelope, PagingCursor, Permission, PrincipalId,
    QueryBudget, Relation, ScopeId,
};
use diskgraph_engine::{Engine, EngineConfig, EngineError};

mod error_reply;
mod html;
mod installer;
mod local;
#[cfg(test)]
mod query_terminal_tests;
mod relation_reply;
mod snapshot_reply;
mod tui;
mod tui_frame_reader;
mod tui_request;

use local::LocalIdentity;

/// DiskGraph — shared Rust file-relationship engine (read-only CLI surface).
#[derive(Parser)]
#[command(
    name = "diskgraph",
    version,
    about,
    after_long_help = include_str!("../QUICKSTART.md"),
    disable_help_subcommand = false
)]
struct Cli {
    /// Data directory holding diskgraph.sqlite and diskgraph-control.sqlite.
    #[arg(long, default_value = "diskgraph-data", global = true)]
    data_dir: PathBuf,
    /// Machine-readable output; stdout carries JSON only, logs go to stderr.
    #[arg(long, global = true)]
    json: bool,
    /// Principal the CLI acts as (default single-user identity).
    #[arg(long, global = true, default_value = "local-user")]
    principal: String,
    /// Hard node ceiling for one scan (RT-04). A walk past it refuses to
    /// publish instead of returning partial data silently.
    #[arg(long, global = true, default_value_t = 2_000_000)]
    max_nodes_per_scan: u64,
    /// Encoded metadata staging budget for one scan (default 2 GiB).
    /// Observed file sizes do not consume this storage budget.
    #[arg(long, global = true, default_value_t = 2 << 30)]
    max_staging_bytes: u64,
    /// Measure apparent length instead of allocated blocks (disktree -a).
    #[arg(short = 'a', long, global = true)]
    apparent_size: bool,
    /// Skip dotfiles and dot-directories (disktree -H).
    #[arg(short = 'H', long, global = true)]
    no_hidden: bool,
    /// Stay on the root's volume (disktree -x; this is the default).
    #[arg(short = 'x', long, global = true)]
    one_filesystem: bool,
    /// Cross filesystem boundaries (disktree -X).
    #[arg(short = 'X', long, global = true)]
    cross_filesystems: bool,
    /// Stop descending past this depth (disktree -d); totals below it are
    /// recorded as unknown rather than estimated.
    #[arg(short = 'd', long, global = true)]
    depth: Option<usize>,
    /// Count a hardlinked file once per link instead of once (disktree's
    /// dedup_hardlinks=false; the default matches disktree's true).
    #[arg(long, global = true)]
    no_dedup_hardlinks: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// C01: manage registered scopes (admin).
    #[command(
        after_help = "EXAMPLES:\n  diskgraph scope add --root ~/projects --data-dir ~/.diskgraph\n  diskgraph scope list --data-dir ~/.diskgraph\n\nRemoving a scope never deletes a file; it only stops tracking."
    )]
    Scope {
        #[command(subcommand)]
        action: ScopeAction,
    },
    /// C02: create a durable index job for a scope.
    #[command(
        after_help = "EXAMPLES:\n  diskgraph index --scope <scope-id> --data-dir ~/.diskgraph --wait\n  diskgraph index --scope <scope-id> --data-dir ~/.diskgraph -a --max-nodes-per-scan 10000000"
    )]
    Index {
        /// Scope ID returned by `scope add`.
        #[arg(long)]
        scope: String,
        /// Wait for the job to finish and report its terminal state.
        #[arg(long)]
        wait: bool,
    },
    /// C03: explicit controlled rescan of a registered scope.
    Sync {
        #[arg(long)]
        scope: String,
        #[arg(long)]
        wait: bool,
    },
    /// C04/C26: show a job's durable state.
    Status {
        #[arg(long)]
        job: String,
    },
    /// C05: list snapshots of a scope.
    Snapshots {
        #[arg(long)]
        scope: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: u64,
        #[arg(long, default_value_t = 0)]
        offset: u64,
        #[command(subcommand)]
        action: Option<SnapshotAction>,
    },
    /// Depth-bounded tree view of a published revision (disktree-style JSON).
    #[command(
        after_help = "EXAMPLES:\n  diskgraph tree --scope <scope-id> --data-dir ~/.diskgraph --depth 4\n  diskgraph tree --scope <scope-id> --data-dir ~/.diskgraph --min-bytes 100000000 --json\n\nChildren come back largest first. A cut-off node reports truncated with its\nchild count, so a shallow tree is never mistaken for a whole one."
    )]
    Tree {
        #[arg(long)]
        scope: String,
        /// Explicit revision; the scope's latest revision by default.
        #[arg(long)]
        revision: Option<String>,
        /// Expand to this depth (1 = root only).
        #[arg(long, default_value_t = 3)]
        depth: usize,
        /// Write a self-contained interactive HTML treemap here instead of
        /// JSON (no network, no build step; open it in any browser).
        #[arg(long, value_name = "PATH")]
        html: Option<PathBuf>,
        /// Replace every directory name with a stable pseudonym (home,
        /// dir-01, ...) so the report can be shared without disclosing
        /// real project or user names. Applies to --html and --json.
        #[arg(long)]
        anonymize: bool,
        /// Hide children whose subtree is smaller than this many bytes.
        #[arg(long, default_value_t = 0)]
        min_bytes: u64,
    },
    /// C10: load the published revision of a scope and report root facts.
    Node {
        #[arg(long)]
        scope: String,
    },
    /// C11/C12: list or rank direct children of a node (bounded).
    Children {
        #[arg(long)]
        scope: String,
        #[arg(long, default_value_t = 1)]
        parent_id: u64,
        #[arg(long, default_value_t = 50)]
        limit: u64,
        #[arg(long, default_value_t = 0)]
        offset: u64,
        /// Only children reporting at least this many bytes.
        #[arg(long)]
        min_bytes: Option<u64>,
        /// Only children whose size could not be reported.
        #[arg(long)]
        unknown_only: bool,
    },
    /// C14: explain one entity with its evidence (relations read-only).
    Explain {
        #[arg(long)]
        scope: String,
        #[arg(long)]
        revision: String,
        #[arg(long)]
        entity: String,
        #[arg(long, default_value_t = 300)]
        limit: u64,
        #[arg(long)]
        after_edge: Option<String>,
    },
    /// C13: typed relations of one entity.
    Related {
        #[arg(long)]
        scope: String,
        #[arg(long)]
        revision: String,
        #[arg(long)]
        entity: String,
        /// Restrict to one registered relation type.
        #[arg(long)]
        relation: Option<String>,
        /// Outgoing (source) edges; omit for incoming.
        #[arg(long)]
        outgoing: bool,
        #[arg(long, default_value_t = 300)]
        limit: u64,
        #[arg(long)]
        after_edge: Option<String>,
    },
    /// C12: largest direct children of a node (bounded, explicit size kind).
    Top {
        #[arg(long)]
        scope: String,
        #[arg(long, default_value_t = 1)]
        parent_id: u64,
        #[arg(long, default_value_t = 20)]
        limit: u64,
    },
    /// C07: growth of one locator between two snapshots (v1 semantics).
    Growth {
        #[arg(long)]
        scope: String,
        #[arg(long)]
        before: String,
        #[arg(long)]
        after: String,
        /// Path under the root to compare; the root itself by default.
        #[arg(long, default_value = "")]
        path: String,
    },
    /// C06: differences between two comparable snapshots; renames are never
    /// inferred (they show as removal + addition).
    Changes {
        #[arg(long)]
        scope: String,
        #[arg(long)]
        before: String,
        #[arg(long)]
        after: String,
    },
    /// Grant or withdraw the right to read file contents inside one scope.
    ///
    /// Registering a scope hands out index, metadata and view rights and
    /// nothing more: reading a file's bytes is a different question from
    /// reading its size, and a grant nobody asked for is the kind that
    /// outlives its reason. `--content-read` is what
    /// `compare --verify-content` needs before it can read anything.
    #[command(
        after_help = "EXAMPLES:\n  diskgraph grant --scope project --content-read\n  diskgraph grant --scope project --revoke-content-read"
    )]
    Grant {
        #[arg(long)]
        scope: String,
        /// Allow reading file contents inside this scope.
        #[arg(long, conflicts_with = "revoke_content_read")]
        content_read: bool,
        /// Take that right back.
        #[arg(long)]
        revoke_content_read: bool,
    },
    /// Compare two trees, and plan a sync between them.
    ///
    /// The roots may be entirely different, which is what `changes` refuses:
    /// this answers "what does this release build have that the working copy
    /// does not", not "what happened to one directory over time".
    ///
    /// With `--plan` the same comparison becomes the steps a sync would take,
    /// in the direction `--from` to `--to`. It is still only a plan: nothing
    /// is written, moved or deleted, and no argument makes it.
    #[command(
        after_help = "EXAMPLES:\n  diskgraph compare --from ./release --to ./worktree\n  diskgraph compare --from-scope release --to-scope worktree --limit 40\n  diskgraph compare --from a --to b --only-differences --json\n  diskgraph compare --from ./release --to ./worktree --plan --method mirror\n\nPlain, each row is a path: only-in-from, only-in-to, different, or same.\nWith --plan, those rows become copy and delete steps.\n\nThe verdict always says which test produced it - size and timestamp, never\ncontents, unless --verify-content was asked for. 'same' means 'same to the\ndepth tested', not 'byte-identical'."
    )]
    Compare {
        /// The tree being compared from.
        #[arg(long, conflicts_with = "from_scope")]
        from: Option<String>,
        /// The tree being compared to.
        #[arg(long, conflicts_with = "to_scope")]
        to: Option<String>,
        /// Take a side from a scope's latest revision instead.
        #[arg(long, conflicts_with = "from")]
        from_scope: Option<String>,
        #[arg(long, conflicts_with = "to")]
        to_scope: Option<String>,
        /// Plan a sync rather than report a comparison: turns the rows into
        /// the copy and delete steps a sync from --from to --to would take.
        #[arg(long)]
        plan: bool,
        /// How the --plan should treat what is already there: update (copy
        /// only), update-both, or mirror (also delete what --to has that
        /// --from does not).
        #[arg(long, default_value = "mirror", value_parser = ["update", "update-both", "mirror"])]
        method: String,
        /// Show only rows that differ; the default shows every path.
        #[arg(long)]
        only_differences: bool,
        /// Rows to print; the summary always covers the whole comparison.
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Seconds two timestamps may differ and still count as the same file.
        #[arg(long, default_value_t = 2)]
        tolerance: i64,
        /// Read the contents of rows the metadata pass called the same, and
        /// report what that shows. Needs the content read grant.
        #[arg(long)]
        verify_content: bool,
        /// Files --verify-content may read.
        #[arg(long, default_value_t = 256)]
        verify_files: u64,
        /// Bytes --verify-content may read per file; a larger file is left
        /// unverified, because hashing it costs more than copying it.
        #[arg(long, default_value_t = 67108864)]
        verify_bytes_per_file: u64,
    },
    /// C09: name/path pattern search inside a published revision (bounded).
    Search {
        #[arg(long)]
        scope: String,
        #[arg(long)]
        pattern: String,
        #[arg(long, default_value_t = 0)]
        offset: u64,
        #[arg(long, default_value_t = 20)]
        limit: u64,
        /// Cursor from a previous page; verified against the same context.
        #[arg(long)]
        cursor: Option<String>,
    },
    /// C08: bounded directory/relation summary with coverage.
    Explore {
        #[arg(long)]
        scope: String,
        #[arg(long, default_value_t = 1)]
        node_id: u64,
        #[arg(long, default_value_t = 2)]
        max_depth: usize,
        #[arg(long, default_value_t = 100)]
        max_nodes: usize,
    },
    /// C15: bounded forward impact following relation-specific directions.
    Impact {
        #[arg(long)]
        scope: String,
        #[arg(long)]
        revision: String,
        #[arg(long)]
        entity: String,
        #[arg(long, default_value_t = 2)]
        max_depth: usize,
        #[arg(long, default_value_t = 300)]
        max_edges: usize,
    },
    /// C16: conservative review candidates from the v1 projection.
    Candidates {
        #[arg(long)]
        scope: String,
        #[arg(long, default_value_t = 0)]
        target_bytes: u64,
    },
    /// C17/C18: content inspection (P7) — not enabled in this build.
    Duplicates,
    /// C18: bounded content read (P7) — not enabled in this build.
    Read {
        #[arg(long)]
        scope: String,
    },
    /// C19: move (P5) — plan-only, not enabled in this build.
    Move {
        #[arg(long)]
        scope: String,
    },
    /// C20: copy (P5) — plan-only, not enabled in this build.
    Copy {
        #[arg(long)]
        scope: String,
    },
    /// C21: trash (P5) — not enabled in this build.
    Trash {
        #[arg(long)]
        scope: String,
    },
    /// C22: restore (P5) — not enabled in this build.
    Restore {
        #[arg(long)]
        scope: String,
    },
    /// C23: purge (P6) — irreversible, not enabled in this build.
    Purge {
        #[arg(long)]
        scope: String,
    },
    /// C24: plan (P5) — not enabled in this build.
    Plan {
        #[command(subcommand)]
        action: PlanAction,
    },
    /// C25: apply (P5) — the only executor, not enabled in this build.
    Apply {
        #[arg(long)]
        scope: String,
    },
    /// C26: operations (P5) — not enabled in this build.
    Operations {
        #[command(subcommand)]
        action: OperationsAction,
    },
    /// C27: serve the MCP transport (stdio, or Streamable HTTP on loopback).
    Serve {
        /// Transport to listen on: stdio or streamable-http.
        #[arg(long, default_value = "stdio")]
        transport: String,
        /// Tool profile to advertise.
        #[arg(long, default_value = "read-full")]
        profile: String,
        /// Address to bind for streamable-http; loopback only.
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Port for streamable-http; 0 picks a free one.
        #[arg(long, default_value_t = 0)]
        port: u16,
        /// Remote bearer-token verifier: issuer, audience and HS256 key.
        #[arg(long, num_args = 3, value_names = ["ISSUER", "AUDIENCE", "KEY"])]
        auth: Option<Vec<String>>,
        /// Read the verifier key from a protected file; no secret enters process arguments.
        #[arg(long, num_args = 3, value_names = ["ISSUER", "AUDIENCE", "PATH"], conflicts_with = "auth")]
        auth_key_file: Option<Vec<String>>,
        /// Allowed browser Origin; repeat for multiple origins.
        #[arg(long)]
        allowed_origin: Vec<String>,
        /// Permit a null Origin when the caller has a valid bearer token.
        #[arg(long)]
        allow_null_origin: bool,
        /// Assert that TLS or an encrypted tunnel terminates before this listener.
        #[arg(long)]
        secure_transport: bool,
        /// Trusted reverse proxy address; repeat for multiple proxies.
        #[arg(long)]
        trusted_proxy: Vec<String>,
    },
    /// C28: register this binary into a client configuration (P3).
    #[command(subcommand)]
    Install(InstallCommand),
    /// Index the current directory and get an agent ready to use the index.
    ///
    /// Run it where you work: the current directory becomes the indexed
    /// root, the index lives in ./.diskgraph, and the agent instruction
    /// files for the platforms you name are written so an agent knows the
    /// index is there. Re-running on an initialized directory refreshes the
    /// index instead of starting over.
    #[command(
        after_help = "EXAMPLES:\n  diskgraph init                 # index this directory, wire up the detected agents\n  diskgraph init --root ../other --data-dir ./.diskgraph\n  diskgraph init --target codex --target claude --yes\n\nRefuses to index your home directory or a filesystem root without --force:\n  diskgraph init --force"
    )]
    Init {
        /// Directory to index; the current directory by default.
        #[arg(long)]
        root: Option<PathBuf>,
        /// Where the index lives; ./.diskgraph by default.
        #[arg(long, default_value = ".diskgraph")]
        data_dir: PathBuf,
        /// Which agent instruction files to write: claude, codex, kimi,
        /// auto (only the detected ones), all, or none. Repeatable.
        #[arg(long = "target", value_delimiter = ',')]
        targets: Vec<String>,
        /// Answer yes to the agent-detection question.
        #[arg(short, long)]
        yes: bool,
        /// Index even a home directory or a filesystem root.
        #[arg(short, long)]
        force: bool,
        /// Write the instructions for every user instead of this project.
        #[arg(long, value_parser = ["global", "local"], default_value = "local")]
        location: String,
        /// Index the directory but leave the agent files alone.
        #[arg(long)]
        no_instructions: bool,
        /// Stop after the index; same as --no-instructions.
        #[arg(long)]
        index_only: bool,
        /// Take the agent instruction blocks back out. Leaves the index alone.
        #[arg(long)]
        uninstall: bool,
        /// Print the instruction block instead of writing it anywhere.
        #[arg(long)]
        print_only: bool,
    },
    /// du-style size summary over one or more paths.
    ///
    /// Each path is registered as a scope (idempotent - re-running reuses
    /// it), indexed with the same walk the index command runs, and
    /// summarized from its published revision. Missing paths are reported
    /// on stderr and skipped, like du.
    #[command(
        after_help = "EXAMPLES:\n  diskgraph du -h ~/Library/Caches ~/.npm ~/.cache ~/.cargo/registry\n  diskgraph du -h --total ~/workspaces/*\n\nThe summary reads the published revision's root, so the number is the\nallocated-bytes figure the index stores - not an estimate of\nreclaimable space."
    )]
    Du {
        /// Paths to measure.
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Print a combined total across the paths (du -c).
        #[arg(short = 'c', long)]
        total: bool,
    },
    /// Interactive treemap of a published revision (terminal).
    #[command(
        after_help = "EXAMPLES:\n  diskgraph tui --scope <scope-id> --data-dir ~/.diskgraph\n\nLoads one directory level at a time, so a multi-million-node index opens\nimmediately. Keys: arrows or hjkl move, enter descends, esc goes up, s toggles\nthe sort, q quits."
    )]
    Tui {
        #[arg(long)]
        scope: String,
        /// Explicit revision; the scope's latest revision by default.
        #[arg(long)]
        revision: Option<String>,
        /// Rename every directory to a stable pseudonym (home, dir-01, ...)
        /// so a captured session can be shared without disclosing names.
        #[arg(long)]
        anonymize: bool,
    },
    /// C29: read-only diagnostics (P3).
    Doctor,
    /// Policy epoch management: publish a new version or revoke everything.
    #[command(subcommand)]
    Policy(PolicyCommand),
}

#[derive(Subcommand)]
enum PolicyCommand {
    /// Publish a new policy version; old-version grants and cursors expire.
    Publish {
        #[arg(long)]
        version: u64,
    },
    /// Revoke the whole policy; nothing is authorized until republished.
    Revoke,
}

#[derive(Subcommand)]
enum PlanAction {
    Show {
        #[arg(long)]
        scope: String,
    },
    Validate {
        #[arg(long)]
        scope: String,
    },
    Create {
        #[arg(long)]
        scope: String,
    },
}

#[derive(Subcommand)]
enum OperationsAction {
    List {
        #[arg(long)]
        scope: String,
    },
    Show {
        #[arg(long)]
        scope: String,
    },
    Cancel {
        #[arg(long)]
        scope: String,
    },
}

/// Client configuration management (P3). Writes are previewed by default and
/// applied only with `--apply-config`.
#[derive(Subcommand)]
enum InstallCommand {
    /// Register the MCP server into a client configuration.
    Add {
        #[arg(long)]
        client: String,
        /// Path to the client configuration file.
        #[arg(long)]
        config: PathBuf,
        /// Write the change; without this the diff is only reported.
        #[arg(long)]
        apply_config: bool,
    },
    /// Report whether DiskGraph is registered, without changing anything.
    Show {
        #[arg(long)]
        client: String,
        #[arg(long)]
        config: PathBuf,
    },
    /// Remove only the entry DiskGraph owns.
    Remove {
        #[arg(long)]
        client: String,
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        apply_config: bool,
    },
}

#[derive(Subcommand)]
enum ScopeAction {
    /// Register (or idempotently return) a scope for a native root.
    Add {
        #[arg(long)]
        root: PathBuf,
    },
    /// List registered scopes.
    List,
    /// Show one scope.
    Show {
        #[arg(long)]
        scope: String,
    },
    /// Revoke a scope; never deletes files or history.
    Remove {
        #[arg(long)]
        scope: String,
    },
}

/// Catalog families this build deliberately does not serve yet: they return
/// the contract's `unsupported` business result (exit 6) rather than an
/// "unknown command" parse error, so callers can tell "not delivered" from
/// "broken" (CMD-02). The stage names the delivery that owns the family.
fn unsupported(catalog_id: &str, stage: &str) -> EngineError {
    eprintln!("{catalog_id} is not enabled in this read-only build (planned for {stage})");
    EngineError::Business(BusinessError::Unsupported)
}

#[cfg(windows)]
fn main() -> ExitCode {
    // Windows 的默认主线程栈无法容纳 Debug 构建的大型命令分发栈帧。
    // 显式预留栈空间，让调试二进制与 Release 二进制使用同一业务路径。
    match std::thread::Builder::new()
        .name("diskgraph-cli".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(cli_main)
    {
        Ok(worker) => worker.join().unwrap_or(ExitCode::from(10)),
        Err(error) => {
            eprintln!("diskgraph: cannot start CLI worker: {error}");
            ExitCode::from(10)
        }
    }
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    cli_main()
}

fn cli_main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::from(0),
        Err(error) => {
            let business = engine_business(&error);
            if std::env::args().any(|arg| arg == "--json") {
                println!("{}", error_reply::line(&error));
            } else {
                eprintln!("{error}");
            }
            ExitCode::from(business.exit_code())
        }
    }
}

fn engine_business(error: &EngineError) -> BusinessError {
    match error {
        EngineError::Business(business) => *business,
        EngineError::Store(store) => match store {
            diskgraph_store::StoreError::SnapshotNotFound(_)
            | diskgraph_store::StoreError::ScopeNotFound(_)
            | diskgraph_store::StoreError::JobNotFound(_)
            | diskgraph_store::StoreError::RevisionNotFound(_) => BusinessError::NotFound,
            diskgraph_store::StoreError::Conflict(_) | diskgraph_store::StoreError::StaleOwner => {
                BusinessError::Conflict
            }
            diskgraph_store::StoreError::RetentionViolation(_) => BusinessError::Conflict,
            diskgraph_store::StoreError::BudgetExceeded => BusinessError::BudgetExceeded,
            error if error.is_interrupted() => BusinessError::BudgetExceeded,
            _ => BusinessError::InternalError,
        },
        EngineError::Io(_) | EngineError::Poisoned => BusinessError::InternalError,
    }
}

/// The scan behavior every command records, taken from the global flags.
/// The snapshot stores these verbatim, so two revisions are comparable only
/// when they were taken with the same ones.
fn scan_options_from(cli: &Cli) -> diskgraph_disktree_core::scan::ScanOptions {
    diskgraph_disktree_core::scan::ScanOptions {
        apparent_size: cli.apparent_size,
        follow_links: false,
        include_hidden: !cli.no_hidden,
        one_filesystem: !cli.cross_filesystems || cli.one_filesystem,
        max_depth: cli.depth,
        dedup_hardlinks: !cli.no_dedup_hardlinks,
        ..diskgraph_disktree_core::scan::ScanOptions::default()
    }
}

/// Why a directory is a bad indexing root, or `None` when it is fine.
///
/// A home directory is hundreds of gigabytes and a filesystem root is larger,
/// and indexing either writes an index the size of the answer. That is a
/// legitimate thing to want, so this refuses with a reason rather than
/// allowing it quietly; `--force` is the way through.
fn unsafe_root_reason(root: &Path) -> Option<String> {
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
        && root == home
    {
        return Some(format!("the home directory {}", home.display()));
    }
    if root.parent().is_none() {
        return Some(format!("the filesystem root {}", root.display()));
    }
    None
}

/// The data directory resolved against the working directory, so `init`
/// writes the index where the user is standing rather than wherever the
/// process happened to be launched from.
fn absolute_data_dir(data_dir: &Path) -> PathBuf {
    if data_dir.is_absolute() {
        return data_dir.to_path_buf();
    }
    std::env::current_dir().map_or_else(|_| data_dir.to_path_buf(), |cwd| cwd.join(data_dir))
}

/// The plan a `compare --plan` produced, in the shape a caller reads: the
/// counts first, so a caller can decide whether to look at the steps at all.
fn plan_to_json(plan: &diskgraph_core::SyncPlan, limit: usize) -> serde_json::Value {
    let steps: Vec<serde_json::Value> = plan
        .actions
        .iter()
        .take(limit)
        // The actions are plain data with derived Serialize; a failure would
        // be a bug, not a runtime condition, so the fallback keeps the
        // envelope well formed.
        .map(|action| serde_json::to_value(action).unwrap_or(serde_json::Value::Null))
        .collect();
    serde_json::json!({
        "method": plan.method.name(),
        "deletes": plan.deletes,
        "copies": plan.copies(),
        "deletions": plan.deletions(),
        "copied_bytes": plan.copied_bytes,
        "deleted_bytes": plan.deleted_bytes,
        "unresolved": plan.unresolved,
        "excluded": plan.excluded,
        "steps_shown": steps.len(),
        "steps": steps,
    })
}

/// One side of a comparison, given either as a revision or as a scope whose
/// latest revision is meant. The scope comes back too, because reading a
/// file's contents is a grant held per scope and a bare revision carries none.
fn resolve_side(
    engine: &Engine,
    revision: Option<&str>,
    scope: Option<&str>,
    side: &str,
    deadline: std::time::Instant,
) -> Result<(String, Option<ScopeId>), EngineError> {
    if let Some(revision) = revision {
        return Ok((revision.to_owned(), None));
    }
    let Some(scope) = scope else {
        eprintln!("diskgraph: the {side} side needs --{side} or --{side}-scope");
        return Err(EngineError::Business(BusinessError::InvalidArgument));
    };
    let scope_id = ScopeId::new(scope.to_owned())
        .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
    engine.scope(&scope_id)?;
    let revision = engine
        .latest_revision_until(&scope_id, deadline)?
        .ok_or(EngineError::Business(BusinessError::NotIndexed))?;
    Ok((revision, Some(scope_id)))
}

/// Registers the root, indexes it, and returns what it found.
///
/// The result is handed back rather than printed: `init` reports the index and
/// the instruction files in one envelope, because a caller parsing the output
/// should not have to reassemble it from two lines.
fn init_index(
    engine: &Engine,
    root: &Path,
    principal: &PrincipalId,
    authorizer: &dyn Authorizer,
) -> Result<serde_json::Value, EngineError> {
    let started = std::time::Instant::now();
    let scope_id = engine.register_scope(root, principal, authorizer)?;
    // register_scope granted this principal scope-local rights in the policy
    // store; the in-memory authorizer is a snapshot from process start, so
    // reload it before asking for the job - otherwise the scan is refused by
    // the very scope it is about to measure.
    let authorizer = &crate::local::LocalIdentity::load(engine)?;
    // A second init is a rescan, not a second scope: the same root maps to
    // the same scope id, and index_scope is what republishes it.
    let job = engine.index_scope(&scope_id, principal, authorizer)?;
    let owner = format!("cli-{principal}");
    let finished = match engine.run_job(&job.job_id, &owner) {
        Ok(record) => record,
        Err(EngineError::Store(diskgraph_store::StoreError::Conflict(_)))
        | Err(EngineError::Store(diskgraph_store::StoreError::StaleOwner)) => {
            wait_for_terminal(engine, &job.job_id, &owner)?
        }
        Err(error) => return Err(error),
    };
    ensure_completed(&finished)?;
    let revision = Some(engine.revision_for_job(&finished.job_id, principal, authorizer)?);
    let root_node = revision
        .as_deref()
        .map(|revision| engine.revision_root_node(revision))
        .transpose()?;
    let index_bytes = std::fs::metadata(engine.data_dir().join("diskgraph.sqlite"))
        .map(|meta| meta.len())
        .unwrap_or(0);
    Ok(serde_json::json!({
        "root": root.display().to_string(),
        "data_dir": engine.data_dir().display().to_string(),
        "scope_id": scope_id.as_str(),
        "revision_id": revision,
        "state": format!("{:?}", finished.state).to_ascii_lowercase(),
        "files": root_node.as_ref().map(|node| node.files),
        "directories": root_node.as_ref().map(|node| node.directories),
        "subtree_bytes": root_node.as_ref().map(|node| node.subtree_bytes),
        "index_bytes": index_bytes,
        "elapsed_ms": started.elapsed().as_millis() as u64,
    }))
}

/// Which agents to write instructions for. `auto` is the default and covers
/// only the platforms found on this machine; an empty list means the user
/// asked for nothing, which is not the same as asking for auto.
fn resolve_targets(requested: &[String]) -> Result<Vec<installer::Target>, EngineError> {
    let wanted = if requested.is_empty() {
        vec!["auto".to_owned()]
    } else {
        requested.to_vec()
    };
    let mut out: Vec<installer::Target> = Vec::new();
    for value in &wanted {
        match value.to_ascii_lowercase().as_str() {
            "auto" => out.extend(
                installer::Target::ALL
                    .into_iter()
                    .filter(|target| target.detected()),
            ),
            "all" => out.extend(installer::Target::ALL),
            "none" => {}
            other => out.extend(installer::Target::parse_list(other).map_err(|message| {
                eprintln!("diskgraph: {message}");
                EngineError::Business(BusinessError::InvalidArgument)
            })?),
        }
    }
    out.dedup();
    Ok(out)
}

fn run(cli: Cli) -> Result<(), EngineError> {
    let deadline = diskgraph_core::query_deadline(QueryBudget::default())?;
    // One knob drives both budget layers: the per-node charged ScanBudget
    // must not be tighter than the hard refusal ceiling, or a caller raising
    // the ceiling would still stop at the old charged limit (RT-02/RT-04).
    let engine = std::sync::Arc::new(
        Engine::open(EngineConfig {
            data_dir: cli.data_dir.clone(),
            max_nodes_per_scan: cli.max_nodes_per_scan,
            scan_budget: diskgraph_core::ScanBudget {
                max_nodes: cli.max_nodes_per_scan,
                max_staging_bytes: cli.max_staging_bytes,
                ..diskgraph_core::ScanBudget::default()
            },
            // Scan behavior mirrors disktree's own flags exactly: the snapshot
            // records these verbatim, and two snapshots are comparable only when
            // they were taken with the same options.
            scan_options: scan_options_from(&cli),
            ..EngineConfig::default()
        })
        .inspect_err(|_error| {
            // 标记故障发生在打开阶段；具体 SQLite 扩展错误码由原错误保留。
            eprintln!("diskgraph: engine startup failed");
        })?,
    );
    // 单次 CLI 查询不消费扫描队列；--wait 由当前请求执行，远程队列由长期服务执行。
    let principal = PrincipalId::new(cli.principal.clone())
        .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
    // The CLI authorizes against the local policy store; an empty policy is
    // default-deny, and first-run setup grants the local user administration.
    let authorizer = LocalIdentity::load(&engine)?;
    let mut json_output = Vec::new();
    let outcome = dispatch(
        &engine,
        &cli,
        &principal,
        &authorizer,
        &mut json_output,
        deadline,
    );
    if cli.json {
        for line in json_output {
            println!("{line}");
        }
    }
    outcome
}

fn dispatch(
    engine: &Engine,
    cli: &Cli,
    principal: &PrincipalId,
    authorizer: &dyn Authorizer,
    out: &mut Vec<String>,
    deadline: std::time::Instant,
) -> Result<(), EngineError> {
    match &cli.command {
        Command::Scope { action } => match action {
            ScopeAction::Add { root } => {
                let scope_id = engine.register_scope(root, principal, authorizer)?;
                out.push(envelope_line(
                    engine,
                    Ok(serde_json::json!({ "scope_id": scope_id.as_str() })),
                ));
                Ok(())
            }
            ScopeAction::List => {
                let scopes = engine.list_scopes(principal, authorizer)?;
                let ids: Vec<&str> = scopes.iter().map(|scope| scope.scope_id.as_str()).collect();
                out.push(envelope_line(
                    engine,
                    Ok(serde_json::json!({ "scopes": ids })),
                ));
                Ok(())
            }
            ScopeAction::Show { scope } => {
                let scope_id = ScopeId::new(scope.clone())
                    .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
                let record = engine.scope(&scope_id)?;
                out.push(envelope_line(
                    engine,
                    Ok(serde_json::json!({
                        "scope_id": record.scope_id.as_str(),
                        "root_display": record.root.display(),
                        "revoked": record.revoked,
                    })),
                ));
                Ok(())
            }
            ScopeAction::Remove { scope } => {
                let scope_id = ScopeId::new(scope.clone())
                    .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
                engine.revoke_scope(&scope_id, principal, authorizer)?;
                out.push(envelope_line(
                    engine,
                    Ok(serde_json::json!({ "revoked": scope_id.as_str() })),
                ));
                Ok(())
            }
        },
        Command::Index { scope, wait } => {
            run_scan(engine, scope, false, *wait, principal, authorizer, out)
        }
        Command::Sync { scope, wait } => {
            run_scan(engine, scope, true, *wait, principal, authorizer, out)
        }
        Command::Status { job } => {
            let record = engine.job_status(job)?;
            out.push(envelope_line(
                engine,
                Ok(serde_json::json!({
                    "job_id": record.job_id,
                    "state": format!("{:?}", record.state).to_ascii_lowercase(),
                    "scope_id": record.scope_id.as_str(),
                })),
            ));
            Ok(())
        }
        Command::Snapshots {
            scope,
            limit,
            offset,
            action,
        } => {
            if let Some(SnapshotAction::Prune {
                scope,
                keep_last,
                apply,
            }) = action
            {
                let scope_id = ScopeId::new(scope.clone())
                    .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
                let candidates =
                    engine.prune_snapshots(&scope_id, *keep_last, *apply, principal, authorizer)?;
                out.push(envelope_line(
                    engine,
                    Ok(serde_json::json!({"applied":apply, "candidates":candidates})),
                ));
                return Ok(());
            }
            let scope_id = ScopeId::new(
                scope
                    .clone()
                    .ok_or(EngineError::Business(BusinessError::InvalidArgument))?,
            )
            .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            let snapshots =
                engine.list_snapshots(&scope_id, principal, authorizer, *limit, *offset)?;
            out.push(envelope_line(
                engine,
                Ok(serde_json::json!({
                    "snapshots": snapshots.iter().map(|snapshot| serde_json::json!({
                        "snapshot_id": snapshot.id,
                        "captured_at_unix_ms": snapshot.captured_at_unix_ms,
                        "complete": snapshot.coverage.complete,
                    })).collect::<Vec<_>>(),
                })),
            ));
            Ok(())
        }
        Command::Tree {
            scope,
            revision,
            depth,
            min_bytes,
            html,
            anonymize,
        } => {
            let scope_id = ScopeId::new(scope.clone())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            engine.scope(&scope_id)?;
            require_metadata(authorizer, principal, &scope_id)?;
            let revision = match revision {
                Some(revision) => revision.clone(),
                None => engine
                    .latest_revision_until(&scope_id, deadline)?
                    .ok_or(EngineError::Business(BusinessError::NotIndexed))?,
            };
            // The narrow read path: no full-graph materialization. A pre-v4
            // snapshot falls back inside the engine and renders identically.
            let mut view = engine.tree_view_until(
                &scope_id,
                &revision,
                principal,
                authorizer,
                *depth,
                *min_bytes,
                snapshot_reply::budget(),
                deadline,
            )?;
            if *anonymize {
                html::anonymize_tree(&mut view.root, "home");
            }
            if let Some(destination) = html {
                let scope_record = engine.scope(&scope_id)?;
                let root = scope_record
                    .root
                    .raw_bytes()
                    .ok()
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                    .unwrap_or_else(|| scope.clone());
                // A shared report must not disclose the user name in the root
                // path; the anonymized root label replaces it.
                let root_label = if *anonymize {
                    "home".to_owned()
                } else {
                    root.clone()
                };
                let truncated = html::tree_is_truncated(&view.root);
                let page = html::render_page(&view.root, &root_label, &revision, truncated, *depth);
                #[cfg(test)]
                query_terminal_tests::before_reply();
                if !snapshot_reply::finalize(engine, principal, authorizer, &[&revision], deadline)?
                {
                    return Err(BusinessError::BudgetExceeded.into());
                }
                std::fs::write(destination, page)?;
                out.push(envelope_line(
                    engine,
                    Ok(serde_json::json!({
                        "revision_id": revision,
                        "rendered_depth": depth,
                        "written": destination.display().to_string(),
                    })),
                ));
                return Ok(());
            }
            out.push(snapshot_reply::finish(
                engine,
                principal,
                authorizer,
                &[&revision],
                serde_json::json!({
                    "revision_id": revision,
                    "rendered_depth": depth,
                    "tree": view.root,
                }),
                deadline,
            )?);
            Ok(())
        }
        Command::Node { scope } => {
            let scope_id = ScopeId::new(scope.clone())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            // A scope that does not exist is not_found, never a permission
            // probe; only live scopes reach the authorization check.
            engine.scope(&scope_id)?;
            require_metadata(authorizer, principal, &scope_id)?;
            let Some(revision) = engine.latest_revision(&scope_id)? else {
                return Err(EngineError::Business(BusinessError::NotIndexed));
            };
            let root = engine.revision_root_node(&revision)?;
            let coverage = engine.revision_snapshot(&revision)?.coverage;
            out.push(envelope_line(
                engine,
                Ok(serde_json::json!({
                    "revision_id": revision,
                    "node": root,
                    "coverage": coverage,
                })),
            ));
            Ok(())
        }
        Command::Children {
            scope,
            parent_id,
            limit,
            offset,
            min_bytes,
            unknown_only,
        } => {
            let scope_id = ScopeId::new(scope.clone())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            // A scope that does not exist is not_found, never a permission
            // probe; only live scopes reach the authorization check.
            engine.scope(&scope_id)?;
            require_metadata(authorizer, principal, &scope_id)?;
            let Some(revision) = engine.latest_revision(&scope_id)? else {
                return Err(EngineError::Business(BusinessError::NotIndexed));
            };
            if *unknown_only && min_bytes.is_some() {
                return Err(EngineError::Business(BusinessError::InvalidArgument));
            }
            let (items, next_offset, unknown_count) = if *unknown_only {
                let (items, next_offset) = engine
                    .revision_unknown_children_page(&revision, *parent_id, *offset, *limit)?;
                (items, next_offset, 0)
            } else {
                engine.revision_children_page(&revision, *parent_id, *min_bytes, *offset, *limit)?
            };
            out.push(envelope_line(
                engine,
                Ok(serde_json::json!({
                    "revision_id": revision,
                    "size_kind": "allocated",
                    "items": items,
                    "next_offset": next_offset,
                    // Nodes whose size could not be reported are counted, not
                    // silently dropped or treated as zero.
                    "unknown_size_count": unknown_count,
                })),
            ));
            Ok(())
        }
        Command::Explain {
            scope,
            revision,
            entity,
            limit,
            after_edge,
        } => {
            let scope_id = ScopeId::new(scope.clone())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            engine.authorize_revision_until(
                Some(&scope_id),
                revision,
                principal,
                authorizer,
                deadline,
            )?;
            let data = engine.explain_bounded_until(
                revision,
                entity,
                after_edge.as_deref(),
                *limit,
                QueryBudget::default(),
                principal,
                authorizer,
                deadline,
            )?;
            out.push(relation_reply::finish(
                engine, principal, authorizer, revision, data, deadline,
            )?);
            Ok(())
        }
        Command::Related {
            scope,
            revision,
            entity,
            relation,
            outgoing,
            limit,
            after_edge,
        } => {
            let scope_id = ScopeId::new(scope.clone())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            engine.authorize_revision_until(
                Some(&scope_id),
                revision,
                principal,
                authorizer,
                deadline,
            )?;
            let relation = match relation {
                Some(name) => Some(
                    Relation::parse(name)
                        .ok_or(EngineError::Business(BusinessError::InvalidArgument))?,
                ),
                None => None,
            };
            let data = engine.related_bounded_until(
                revision,
                entity,
                relation,
                *outgoing,
                after_edge.as_deref(),
                *limit,
                QueryBudget::default(),
                principal,
                authorizer,
                deadline,
            )?;
            out.push(relation_reply::finish(
                engine, principal, authorizer, revision, data, deadline,
            )?);
            Ok(())
        }
        Command::Top {
            scope,
            parent_id,
            limit,
        } => {
            let scope_id = ScopeId::new(scope.clone())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            // A scope that does not exist is not_found, never a permission
            // probe; only live scopes reach the authorization check.
            engine.scope(&scope_id)?;
            require_metadata(authorizer, principal, &scope_id)?;
            let Some(revision) = engine.latest_revision(&scope_id)? else {
                return Err(EngineError::Business(BusinessError::NotIndexed));
            };
            let (items, more) = engine.revision_top(&revision, *parent_id, *limit)?;
            out.push(envelope_line(
                engine,
                Ok(serde_json::json!({
                    "revision_id": revision,
                    "size_kind": "allocated",
                    "items": items,
                    "truncated": more.then_some("node_limit"),
                })),
            ));
            Ok(())
        }
        Command::Growth {
            scope,
            before,
            after,
            path,
        } => {
            let scope = ScopeId::new(scope.clone()).map_err(|_| BusinessError::InvalidArgument)?;
            engine.authorize_revision_until(
                Some(&scope),
                before,
                principal,
                authorizer,
                deadline,
            )?;
            engine.authorize_revision_until(
                Some(&scope),
                after,
                principal,
                authorizer,
                deadline,
            )?;
            let growth = engine.growth_between_until(
                before,
                after,
                std::path::Path::new(path),
                snapshot_reply::budget(),
                principal,
                authorizer,
                deadline,
            )?;
            out.push(snapshot_reply::finish(
                engine,
                principal,
                authorizer,
                &[before, after],
                serde_json::json!({
                    "delta_bytes": growth.as_ref().map(|g| g.delta_bytes.to_string()),
                    "comparable": growth.is_some(),
                    "path": if path.is_empty() { ".".to_owned() } else { path.clone() },
                }),
                deadline,
            )?);
            Ok(())
        }
        Command::Grant {
            scope,
            content_read,
            revoke_content_read,
        } => {
            let scope_id = ScopeId::new(scope.clone())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            engine.scope(&scope_id)?;
            let allow = *content_read || !*revoke_content_read;
            engine.set_content_read(&scope_id, principal, allow)?;
            out.push(envelope_line(
                engine,
                Ok(serde_json::json!({
                    "scope_id": scope_id.as_str(),
                    "content_read": if allow { "granted" } else { "revoked" },
                })),
            ));
            Ok(())
        }
        Command::Compare {
            from,
            to,
            from_scope,
            to_scope,
            plan,
            method,
            only_differences,
            limit,
            tolerance,
            verify_content,
            verify_files,
            verify_bytes_per_file,
        } => {
            let (from_revision, from_scope_id) = resolve_side(
                engine,
                from.as_deref(),
                from_scope.as_deref(),
                "from",
                deadline,
            )?;
            let (to_revision, to_scope_id) =
                resolve_side(engine, to.as_deref(), to_scope.as_deref(), "to", deadline)?;

            // Verification needs a scope on each side: the content grant is
            // granted per scope, and reading a path is only allowed inside the
            // scope that covers it.
            // Content is read inside a scope, so verification needs one on
            // each side; a bare revision carries no grant to check against.
            if *verify_content && (from_scope_id.is_none() || to_scope_id.is_none()) {
                eprintln!(
                    "diskgraph: --verify-content needs --from-scope and --to-scope: \
                     content is read inside a scope, not from a bare revision"
                );
                return Err(EngineError::Business(BusinessError::InvalidArgument));
            }
            engine.authorize_revision_until(
                from_scope_id.as_ref(),
                &from_revision,
                principal,
                authorizer,
                deadline,
            )?;
            engine.authorize_revision_until(
                to_scope_id.as_ref(),
                &to_revision,
                principal,
                authorizer,
                deadline,
            )?;
            let mut verification = None;

            if *plan {
                let sync_method = diskgraph_core::SyncMethod::parse(method)
                    .ok_or(EngineError::Business(BusinessError::InvalidArgument))?;
                let sync = engine.sync_plan_until(
                    &from_revision,
                    &to_revision,
                    sync_method,
                    *tolerance,
                    snapshot_reply::budget(),
                    principal,
                    authorizer,
                    deadline,
                )?;
                out.push(snapshot_reply::finish_plan(
                    engine,
                    principal,
                    authorizer,
                    &[&from_revision, &to_revision],
                    plan_to_json(&sync, *limit),
                    deadline,
                )?);
                return Ok(());
            }

            let mut report = engine.compare_revisions_until(
                &from_revision,
                &to_revision,
                *tolerance,
                snapshot_reply::budget(),
                principal,
                authorizer,
                deadline,
            )?;
            if *verify_content {
                let budget = diskgraph_engine::verify::VerifyBudget {
                    max_files: *verify_files,
                    max_bytes_per_file: *verify_bytes_per_file,
                };
                let (promoted, summary) = diskgraph_engine::verify::verify_same_rows_until(
                    engine,
                    report,
                    from_scope_id.as_ref().expect("checked above"),
                    to_scope_id.as_ref().expect("checked above"),
                    principal,
                    authorizer,
                    budget,
                    deadline,
                )?;
                report = promoted;
                verification = Some(summary);
            }
            let mut json = report.to_json(Some(*limit));
            if *only_differences {
                let mut shown = 0_usize;
                if let Some(rows) = json
                    .get_mut("rows")
                    .and_then(serde_json::Value::as_array_mut)
                {
                    rows.retain(|row| {
                        row.get("verdict")
                            .and_then(|verdict| verdict.get("status"))
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(|status| status != "same")
                    });
                    rows.truncate(*limit);
                    shown = rows.len();
                }
                if let Some(object) = json.as_object_mut() {
                    object.insert("entries".into(), serde_json::json!(shown));
                }
            }
            if let (Some(summary), Some(object)) = (verification, json.as_object_mut()) {
                object.insert("verification".into(), serde_json::json!(summary));
            }
            out.push(snapshot_reply::finish(
                engine,
                principal,
                authorizer,
                &[&from_revision, &to_revision],
                json,
                deadline,
            )?);
            Ok(())
        }
        Command::Changes {
            scope,
            before,
            after,
        } => {
            let scope = ScopeId::new(scope.clone()).map_err(|_| BusinessError::InvalidArgument)?;
            engine.authorize_revision_until(
                Some(&scope),
                before,
                principal,
                authorizer,
                deadline,
            )?;
            engine.authorize_revision_until(
                Some(&scope),
                after,
                principal,
                authorizer,
                deadline,
            )?;
            let data = engine.revision_changes_until(
                before,
                after,
                snapshot_reply::budget(),
                principal,
                authorizer,
                deadline,
            )?;
            out.push(snapshot_reply::finish(
                engine,
                principal,
                authorizer,
                &[before, after],
                data,
                deadline,
            )?);
            Ok(())
        }
        Command::Search {
            scope,
            pattern,
            offset,
            limit,
            cursor,
        } => {
            let scope_id = ScopeId::new(scope.clone())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            let Some(revision) = engine.latest_revision(&scope_id)? else {
                return Err(EngineError::Business(BusinessError::NotIndexed));
            };
            // A cursor is only honored when it was issued for this same
            // principal, scope, revision, pattern, and sort order (Q-02).
            let pattern_binding = format!("pattern:{pattern}");
            let context = CursorContext {
                principal_binding: &cli.principal,
                scope_id: scope_id.as_str(),
                revision_id: &revision,
                filter_binding: &pattern_binding,
                sort_binding: "name_asc,id_asc,keyset_v2",
                policy_version: authorizer.policy_version(),
            };
            let cursor = cursor
                .as_ref()
                .map(|encoded| diskgraph_core::SearchCursor::decode(encoded, &context))
                .transpose()
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            engine.authorize_revision(Some(&scope_id), &revision, principal, authorizer)?;
            let reader = engine.revision_reader()?;
            let snapshot = reader.revision(&revision)?.snapshot_id;
            let after = cursor
                .as_ref()
                .map(|cursor| (cursor.last_name.as_str(), cursor.last_id));
            let (items, more) =
                reader.search_page(&snapshot, pattern, after, *offset, (*limit).clamp(1, 100))?;
            let consumed = cursor
                .as_ref()
                .map_or(*offset, |cursor| cursor.binding.offset)
                .saturating_add(items.len() as u64);
            let next_cursor = items.last().filter(|_| more).map(|last| {
                diskgraph_core::SearchCursor {
                    version: 2,
                    binding: PagingCursor::issue(
                        &context,
                        pattern_binding.clone(),
                        context.sort_binding,
                        consumed,
                    ),
                    last_name: last.name.clone(),
                    last_id: last.id,
                }
                .encode()
            });
            out.push(envelope_line(
                engine,
                Ok(serde_json::json!({
                    "revision_id": revision,
                    "items": items,
                    "next_cursor": next_cursor,
                })),
            ));
            Ok(())
        }
        Command::Explore {
            scope,
            node_id,
            max_depth,
            max_nodes,
        } => {
            let scope_id = ScopeId::new(scope.clone())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            // A scope that does not exist is not_found, never a permission
            // probe; only live scopes reach the authorization check.
            engine.scope(&scope_id)?;
            require_metadata(authorizer, principal, &scope_id)?;
            let Some(revision) = engine.latest_revision(&scope_id)? else {
                return Err(EngineError::Business(BusinessError::NotIndexed));
            };
            let budget = QueryBudget {
                max_depth: *max_depth,
                max_nodes: *max_nodes,
                ..QueryBudget::default()
            };
            let page_limit = budget.max_nodes.min(100);
            let (node, children, more) =
                engine.revision_layer_page(&revision, *node_id, 0, page_limit)?;
            let coverage = engine.revision_snapshot(&revision)?.coverage;
            out.push(envelope_line(
                engine,
                Ok(serde_json::json!({
                    "revision_id": revision,
                    "node": node,
                    "children": children,
                    "coverage": coverage,
                    "truncated": (more || page_limit == 0).then_some("node_limit"),
                })),
            ));
            Ok(())
        }
        Command::Impact {
            scope,
            revision,
            entity,
            max_depth,
            max_edges,
        } => {
            let scope_id = ScopeId::new(scope.clone())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            engine.authorize_revision_until(
                Some(&scope_id),
                revision,
                principal,
                authorizer,
                deadline,
            )?;
            let budget = QueryBudget {
                max_depth: *max_depth,
                max_edges: *max_edges,
                ..QueryBudget::default()
            };
            let answer = engine
                .revision_impact_until(revision, entity, budget, principal, authorizer, deadline)?;
            out.push(relation_reply::finish(
                engine,
                principal,
                authorizer,
                revision,
                serde_json::json!({
                    "revision_id": revision,
                    "entity": entity,
                    "entries": answer.entries.iter().map(|entry| serde_json::json!({
                        "entity_id": entry.entity_id,
                        "relation": entry.relation.wire_name(),
                        "depth": entry.depth,
                    })).collect::<Vec<_>>(),
                    "grants_execution": false,
                    "complete": answer.truncated.is_none(),
                    "truncated": answer.truncated.map(|reason|reason.wire_name()),
                }),
                deadline,
            )?);
            Ok(())
        }
        Command::Duplicates => Err(unsupported("C17", "P7")),
        Command::Read { scope: _ } => Err(unsupported("C18", "P7")),
        Command::Move { scope: _ } => Err(unsupported("C19", "P5")),
        Command::Copy { scope: _ } => Err(unsupported("C20", "P5")),
        Command::Trash { scope: _ } => Err(unsupported("C21", "P5")),
        Command::Restore { scope: _ } => Err(unsupported("C22", "P5")),
        Command::Purge { scope: _ } => Err(unsupported("C23", "P6")),
        Command::Plan { action: _ } => Err(unsupported("C24", "P5")),
        Command::Apply { scope: _ } => Err(unsupported("C25", "P5")),
        Command::Operations { action: _ } => Err(unsupported("C26", "P5")),
        Command::Serve {
            transport,
            profile,
            host,
            port,
            auth,
            auth_key_file,
            allowed_origin,
            allow_null_origin,
            secure_transport,
            trusted_proxy,
        } => {
            let Some(profile) = diskgraph_mcp::protocol::ToolProfile::parse(profile) else {
                return Err(EngineError::Business(BusinessError::InvalidArgument));
            };
            // `serve` execs the MCP server so this process's stdout stays clean
            // and the child owns the transport.
            let binary = std::env::current_exe()
                .map_err(|error| EngineError::Store(diskgraph_store::StoreError::Io(error)))?
                .with_file_name(if cfg!(windows) {
                    "diskgraph-mcp.exe"
                } else {
                    "diskgraph-mcp"
                });
            let mut child = std::process::Command::new(binary);
            child
                .arg("--data-dir")
                .arg(&cli.data_dir)
                .arg("--profile")
                .arg(profile.wire_name())
                .arg("--transport")
                .arg(transport)
                .arg("--host")
                .arg(host)
                .arg("--port")
                .arg(port.to_string());
            if let Some(auth) = auth {
                child.arg("--auth").args(auth);
            }
            if let Some(auth_key_file) = auth_key_file {
                child.arg("--auth-key-file").args(auth_key_file);
            }
            for origin in allowed_origin {
                child.arg("--allowed-origin").arg(origin);
            }
            if *allow_null_origin {
                child.arg("--allow-null-origin");
            }
            if *secure_transport {
                child.arg("--secure-transport");
            }
            for proxy in trusted_proxy {
                child.arg("--trusted-proxy").arg(proxy);
            }
            let status = child
                .status()
                .map_err(|error| EngineError::Store(diskgraph_store::StoreError::Io(error)))?;
            // The child owns the streams; propagate its exit code.
            std::process::exit(status.code().unwrap_or(10) as i32);
        }
        Command::Install(action) => install_tool(action),
        Command::Policy(action) => match action {
            PolicyCommand::Publish { version } => {
                engine.publish_policy_version(*version, principal, authorizer)?;
                out.push(envelope_line(
                    engine,
                    Ok(serde_json::json!({"published_version": version})),
                ));
                Ok(())
            }
            PolicyCommand::Revoke => {
                engine.revoke_policy(principal, authorizer)?;
                out.push(envelope_line(
                    engine,
                    Ok(serde_json::json!({"revoked": true})),
                ));
                Ok(())
            }
        },
        Command::Du { paths, total } => {
            let mut rows: Vec<(PathBuf, u64, String)> = Vec::new();
            let mut failures = 0_usize;
            for path in paths {
                let canonical = match path.canonicalize() {
                    Ok(canonical) => canonical,
                    Err(_) => {
                        eprintln!(
                            "diskgraph: cannot access {}: No such file or directory",
                            path.display()
                        );
                        failures += 1;
                        continue;
                    }
                };
                let scope_id = match engine.register_scope(&canonical, principal, authorizer) {
                    Ok(scope_id) => scope_id,
                    Err(error) => {
                        eprintln!("diskgraph: {}: {error}", path.display());
                        failures += 1;
                        continue;
                    }
                };
                // register_scope granted this principal scope-local rights in
                // the policy store; the in-memory authorizer is a snapshot
                // from process start, so reload it before indexing.
                let authorizer = &crate::local::LocalIdentity::load(engine)?;
                let job = match engine.index_scope(&scope_id, principal, authorizer) {
                    Ok(job) => job,
                    Err(error) => {
                        eprintln!("diskgraph: {}: {error}", path.display());
                        failures += 1;
                        continue;
                    }
                };
                // 其他长期服务可能已认领任务；等待或接管该任务，不运行其他队列项。
                let completed = match engine.run_job(&job.job_id, "du") {
                    Ok(record) => Ok(record),
                    Err(EngineError::Store(diskgraph_store::StoreError::Conflict(_)))
                    | Err(EngineError::Store(diskgraph_store::StoreError::StaleOwner)) => {
                        wait_for_terminal(engine, &job.job_id, "du")
                    }
                    Err(error) => Err(error),
                };
                match completed {
                    Ok(record) if record.state == diskgraph_store::JobState::Completed => {}
                    Ok(record) => {
                        eprintln!(
                            "diskgraph: {}: job ended {:?}",
                            path.display(),
                            record.state
                        );
                        failures += 1;
                        continue;
                    }
                    Err(error) => {
                        eprintln!("diskgraph: {}: {error}", path.display());
                        failures += 1;
                        continue;
                    }
                }
                let Some(revision) = engine.latest_revision(&scope_id)? else {
                    failures += 1;
                    continue;
                };
                let root = engine.revision_root_node(&revision)?;
                rows.push((canonical, root.subtree_bytes, scope_id.as_str().to_owned()));
            }
            if failures > 0 && rows.is_empty() {
                return Err(EngineError::Business(BusinessError::NotFound));
            }
            let grand_total: u64 = rows.iter().map(|row| row.1).sum();
            let mut data = serde_json::Map::new();
            let _ = total;
            for (path, bytes, _) in &rows {
                data.insert(
                    path.display().to_string(),
                    serde_json::json!({ "bytes": bytes }),
                );
            }
            if !cli.json {
                // du -sh shape: size, tab, path - readable without a parser.
                for (path, bytes, _) in &rows {
                    println!(
                        "{}\t{}",
                        diskgraph_core::treemap::human_bytes(*bytes),
                        path.display()
                    );
                }
                if *total && rows.len() > 1 {
                    println!(
                        "{}\ttotal",
                        diskgraph_core::treemap::human_bytes(grand_total)
                    );
                }
                if failures > 0 {
                    eprintln!("diskgraph: {failures} path(s) could not be measured");
                }
                return Ok(());
            }
            let mut payload = serde_json::Map::new();
            payload.insert("sizes".to_owned(), serde_json::Value::Object(data));
            if *total && rows.len() > 1 {
                payload.insert("total_bytes".to_owned(), serde_json::json!(grand_total));
            }
            if failures > 0 {
                payload.insert("failed_paths".to_owned(), serde_json::json!(failures));
            }
            out.push(envelope_line(
                engine,
                Ok(serde_json::Value::Object(payload)),
            ));
            Ok(())
        }
        Command::Tui {
            scope,
            revision,
            anonymize,
        } => {
            let scope_id = ScopeId::new(scope.clone())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            engine.scope(&scope_id)?;
            require_metadata(authorizer, principal, &scope_id)?;
            let revision = match revision {
                Some(revision) => revision.clone(),
                None => engine
                    .latest_revision(&scope_id)?
                    .ok_or(EngineError::Business(BusinessError::NotIndexed))?,
            };
            engine.authorize_revision(Some(&scope_id), &revision, principal, authorizer)?;
            tui::run(engine, &revision, principal, authorizer, *anonymize)?;
            Ok(())
        }
        Command::Init {
            root,
            data_dir,
            targets,
            yes,
            force,
            location,
            no_instructions,
            index_only,
            uninstall,
            print_only,
        } => {
            // `init` is the only command that opens a store on a directory
            // the user did not name as a scope, so the root is resolved and
            // checked before the engine is even constructed.
            let root = root
                .clone()
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            let root = std::fs::canonicalize(&root)
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            if !root.is_dir() {
                eprintln!(
                    "diskgraph: init needs a directory, and {} is not one",
                    root.display()
                );
                return Ok(());
            }
            if !*force && let Some(reason) = unsafe_root_reason(&root) {
                eprintln!("diskgraph: refusing to index {reason}");
                eprintln!("diskgraph: pass --force if that is what you meant");
                return Ok(());
            }
            // The data directory is resolved against the directory the user is
            // standing in, so `init` writes where they are looking.
            let data_dir = absolute_data_dir(data_dir);
            if data_dir != cli.data_dir {
                // A different directory than the global flag means this
                // command needs its own store; every other command takes one
                // --data-dir for all of them, so this is a one-off.
                eprintln!(
                    "diskgraph: init will use {} for the index; pass the same \
                     --data-dir to the other commands",
                    data_dir.display()
                );
            }
            let engine = std::sync::Arc::new(Engine::open(EngineConfig {
                data_dir: data_dir.clone(),
                max_nodes_per_scan: cli.max_nodes_per_scan,
                scan_budget: diskgraph_core::ScanBudget {
                    max_nodes: cli.max_nodes_per_scan,
                    max_staging_bytes: cli.max_staging_bytes,
                    ..diskgraph_core::ScanBudget::default()
                },
                scan_options: scan_options_from(cli),
                ..EngineConfig::default()
            })?);
            let summary = init_index(&engine, &root, principal, authorizer)?;
            let chosen = resolve_targets(targets)?;
            let global = location == "global";
            let body = installer::instruction_body(&installer::locale_from_env());
            if *print_only {
                // The block a target would get, on stdout and nowhere else:
                // the way to read it before letting us write into a file you
                // have been keeping for a year.
                out.push(envelope_line(
                    &engine,
                    Ok(serde_json::json!({
                        "index": summary,
                        "instructions": body,
                        "targets": chosen.iter().map(|target| target.name()).collect::<Vec<_>>(),
                    })),
                ));
                return Ok(());
            }
            if *uninstall {
                let mut removed = Vec::new();
                for file in installer::instruction_files(&chosen, &root, global) {
                    let outcome = installer::remove_block(&file.path)?;
                    removed.push(serde_json::json!({
                        "target": file.target.name(),
                        "path": file.path.display().to_string(),
                        "outcome": outcome.as_str(),
                    }));
                }
                out.push(envelope_line(
                    &engine,
                    Ok(serde_json::json!({ "index": summary, "instructions": removed })),
                ));
                return Ok(());
            }
            if *no_instructions || *index_only || !*yes {
                if !*no_instructions && !*index_only && !*yes {
                    eprintln!(
                        "diskgraph: index ready; pass --yes to write the agent instruction files"
                    );
                }
                out.push(envelope_line(
                    &engine,
                    Ok(serde_json::json!({ "index": summary })),
                ));
                return Ok(());
            }
            let mut written = Vec::new();
            let mut touched = 0_usize;
            for file in installer::instruction_files(&chosen, &root, global) {
                let outcome = installer::upsert_block(&file.path, body)?;
                touched += usize::from(outcome.wrote());
                written.push(serde_json::json!({
                    "target": file.target.name(),
                    "path": file.path.display().to_string(),
                    "outcome": outcome.as_str(),
                }));
            }
            if touched == 0 && !written.is_empty() {
                // A second init that changes nothing should say so, rather
                // than letting the file list read as if it had been rewritten.
                eprintln!("diskgraph: the agent files already say this; nothing written");
            }
            if let Some(kimi_config) = installer::kimi_mcp_config_path()
                && !installer::kimi_has_diskgraph(&kimi_config)
            {
                eprintln!(
                    "diskgraph: kimi reads MCP servers from {} - add diskgraph there with",
                    kimi_config.display()
                );
                eprintln!("diskgraph:   diskgraph serve --profile read-full");
            }
            out.push(envelope_line(
                &engine,
                Ok(serde_json::json!({ "index": summary, "instructions": written })),
            ));
            Ok(())
        }
        Command::Doctor => {
            let profile = diskgraph_mcp::protocol::ToolProfile::ReadFull;
            let report = diskgraph_mcp::doctor::diagnose(engine, profile);
            // A degraded report is data, not a tool failure.
            out.push(envelope_line(engine, Ok(report.to_json(engine, profile))));
            Ok(())
        }
        Command::Candidates {
            scope,
            target_bytes,
        } => {
            let scope_id = ScopeId::new(scope.clone())
                .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
            // A scope that does not exist is not_found, never a permission
            // probe; only live scopes reach the authorization check.
            engine.scope(&scope_id)?;
            require_metadata(authorizer, principal, &scope_id)?;
            let Some(revision) = engine.latest_revision_until(&scope_id, deadline)? else {
                return Err(EngineError::Business(BusinessError::NotIndexed));
            };
            let answer = engine.review_candidates_until(
                &revision,
                *target_bytes,
                QueryBudget::default(),
                principal,
                authorizer,
                deadline,
            )?;
            let candidates: Vec<_> = answer
                .candidates
                .into_iter()
                .map(|(node, evidence)| {
                    serde_json::json!({
                        "node": node,
                        "evidence": evidence,
                    })
                })
                .collect();
            out.push(relation_reply::finish(
                engine,
                principal,
                authorizer,
                &revision,
                serde_json::json!({
                    "candidates": candidates,
                    "review_only": true,
                    "coverage_complete": answer.coverage_complete,
                    "coverage_observed": answer.coverage_observed,
                    "complete": answer.complete,
                    "truncated": answer.truncated.map(|reason| reason.wire_name()),
                    "selected_bytes": answer.selected_bytes.to_string(),
                    "remaining_bytes": answer.remaining_bytes.to_string(),
                }),
                deadline,
            )?);
            Ok(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_scan(
    engine: &Engine,
    scope: &str,
    sync: bool,
    wait: bool,
    principal: &PrincipalId,
    authorizer: &dyn Authorizer,
    out: &mut Vec<String>,
) -> Result<(), EngineError> {
    let scope_id = ScopeId::new(scope.to_owned())
        .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))?;
    let job = if sync {
        engine.sync_scope(&scope_id, principal, authorizer)?
    } else {
        engine.index_scope(&scope_id, principal, authorizer)?
    };
    if !wait {
        out.push(envelope_line(
            engine,
            Ok(serde_json::json!({ "job_id": job.job_id, "state": "queued" })),
        ));
        return Ok(());
    }
    let owner = format!("cli-{principal}");
    // 其他长期服务可能认领任务；--wait 必须核验该 owner 的实际终态。
    let finished = match engine.run_job(&job.job_id, &owner) {
        Ok(record) => record,
        Err(EngineError::Store(diskgraph_store::StoreError::Conflict(_)))
        | Err(EngineError::Store(diskgraph_store::StoreError::StaleOwner)) => {
            wait_for_terminal(engine, &job.job_id, &owner)?
        }
        Err(error) => return Err(error),
    };
    ensure_completed(&finished)?;
    let revision = engine.revision_for_job(&finished.job_id, principal, authorizer)?;
    out.push(envelope_line(
        engine,
        Ok(serde_json::json!({
            "job_id": finished.job_id,
            "state": format!("{:?}", finished.state).to_ascii_lowercase(),
            "revision_id": revision,
        })),
    ));
    Ok(())
}

/// CLI queries authorize exactly like the MCP surface: metadata reads need
/// metadata:read on the scope they name (P4-5.9).
fn require_metadata(
    authorizer: &dyn Authorizer,
    principal: &PrincipalId,
    scope_id: &ScopeId,
) -> Result<(), EngineError> {
    match authorizer.decide(principal, &Permission::MetadataRead, scope_id) {
        diskgraph_core::Decision::Allowed => Ok(()),
        diskgraph_core::Decision::Denied(_) => {
            Err(EngineError::Business(BusinessError::PermissionDenied))
        }
    }
}

/// C28 install: register, inspect, or remove the MCP server entry. Writes are
/// previewed unless `--apply-config` is given, and removal touches only the
/// entry DiskGraph owns.
fn install_tool(action: &InstallCommand) -> Result<(), EngineError> {
    use diskgraph_mcp::install::{Registration, add, remove, require_client, show};

    let outcome = match action {
        InstallCommand::Add {
            client,
            config,
            apply_config,
        } => {
            require_client(client).map_err(|error| {
                EngineError::Store(diskgraph_store::StoreError::InvalidGraph(error.to_string()))
            })?;
            let registration = Registration::stdio(
                std::env::current_exe()
                    .map_err(|error| EngineError::Store(diskgraph_store::StoreError::Io(error)))?,
                std::env::current_dir().unwrap_or_default(),
            );
            add(config, &registration, *apply_config).map(|applied| applied.to_json("add"))
        }
        InstallCommand::Show { client, config } => {
            require_client(client).map_err(|error| {
                EngineError::Store(diskgraph_store::StoreError::InvalidGraph(error.to_string()))
            })?;
            show(config)
        }
        InstallCommand::Remove {
            client,
            config,
            apply_config,
        } => {
            require_client(client).map_err(|error| {
                EngineError::Store(diskgraph_store::StoreError::InvalidGraph(error.to_string()))
            })?;
            remove(config, *apply_config).map(|applied| applied.to_json("remove"))
        }
    };
    match outcome {
        Ok(payload) => {
            let envelope = Envelope::ok(payload);
            println!("{}", serde_json::to_string(&envelope).unwrap_or_default());
            Ok(())
        }
        Err(error) => Err(EngineError::Store(
            diskgraph_store::StoreError::InvalidGraph(error.to_string()),
        )),
    }
}

/// 等待指定任务；仅在其排队或租约过期时尝试条件认领，不启动后台队列。
fn wait_for_terminal(
    engine: &Engine,
    job_id: &str,
    owner: &str,
) -> Result<diskgraph_store::JobRecord, EngineError> {
    wait_for_terminal_until(
        engine,
        job_id,
        owner,
        std::time::Instant::now() + std::time::Duration::from_secs(120),
    )
}

fn wait_for_terminal_until(
    engine: &Engine,
    job_id: &str,
    owner: &str,
    deadline: std::time::Instant,
) -> Result<diskgraph_store::JobRecord, EngineError> {
    use diskgraph_store::JobState;
    loop {
        let record = engine.job_status(job_id)?;
        if matches!(
            record.state,
            JobState::Completed | JobState::Failed | JobState::Cancelled
        ) {
            ensure_completed(&record)?;
            return Ok(record);
        }
        if std::time::Instant::now() >= deadline {
            return Err(EngineError::Business(BusinessError::Timeout));
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        if record.state == JobState::Queued
            || (record.state == JobState::Running && record.lease_expires_unix_ms <= now_ms)
        {
            let settled = engine.settle_expired_job(job_id)?;
            if matches!(
                settled.state,
                JobState::Completed | JobState::Failed | JobState::Cancelled
            ) {
                ensure_completed(&settled)?;
                return Ok(settled);
            }
            // 数据库条件更新与 fencing 决定唯一 owner；存活 owner 不被抢占。
            // 认领成功后的扫描仍使用 Engine 扫描预算，deadline 仅限制等待。
            match engine.run_job(job_id, owner) {
                Ok(record) => {
                    ensure_completed(&record)?;
                    return Ok(record);
                }
                Err(EngineError::Store(diskgraph_store::StoreError::Conflict(_)))
                | Err(EngineError::Store(diskgraph_store::StoreError::StaleOwner)) => {}
                Err(error) => return Err(error),
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[cfg(test)]
mod scan_terminal_tests;

/// 失败/取消的其他 owner 结果没有持久原因码，诚实返回 partial，不能猜测成功。
fn ensure_completed(record: &diskgraph_store::JobRecord) -> Result<(), EngineError> {
    match record.state {
        diskgraph_store::JobState::Completed => Ok(()),
        diskgraph_store::JobState::Failed | diskgraph_store::JobState::Cancelled => {
            Err(EngineError::Business(BusinessError::Partial))
        }
        _ => Err(EngineError::Business(BusinessError::Conflict)),
    }
}

fn envelope_line(engine: &Engine, result: Result<serde_json::Value, EngineError>) -> String {
    match result {
        Ok(data) => {
            let envelope = Envelope::ok(data).with_ids(engine.server_id().ok(), None, None);
            serde_json::to_string(&envelope).unwrap_or_else(|_| {
                serde_json::to_string(&Envelope::failure(BusinessError::InternalError, "encode"))
                    .unwrap()
            })
        }
        Err(error) => {
            let business = engine_business(&error);
            serde_json::to_string(&Envelope::failure(business, error.to_string())).unwrap()
        }
    }
}
