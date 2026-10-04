//! 完整 CLI 业务命令目录的真实解析对象。
use crate::{
    install_command::InstallCommand, operations_action::OperationsAction, plan_action::PlanAction,
    policy_command::PolicyCommand, scope_action::ScopeAction, snapshot_action::SnapshotAction,
};
use clap::Subcommand;
use std::path::PathBuf;

#[cfg_attr(
    doc,
    doc = "完整 CLI 业务命令目录。来源：DiskGraph 原生 CLI main::Command；无 Java 对应实现。"
)]
#[derive(Subcommand)]
pub(crate) enum Command {
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
        /// Explicit evidence collector; omit for the existing filesystem rescan.
        #[arg(long, value_parser = ["git", "process"], requires_all = ["revision", "node_id"])]
        collector: Option<String>,
        /// 包含固定 Git 目录或进程观察文件的已发布 revision。
        #[arg(long, requires = "collector")]
        revision: Option<String>,
        /// 已发布版本中的 Git 目录或进程观察普通文件节点。
        #[arg(long, requires = "collector", value_parser = clap::value_parser!(u64).range(1..))]
        node_id: Option<u64>,
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
