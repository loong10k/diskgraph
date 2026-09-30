QUICK START

  # 1. register a directory to watch (prints a scope id)
  diskgraph scope add --root ~/projects --data-dir ~/.diskgraph

  # 2. index it and wait for the walk to publish a revision
  diskgraph index --scope <scope-id> --data-dir ~/.diskgraph --wait

  # 3. ask questions (all bounded, all read-only)
  diskgraph top    --scope <scope-id> --data-dir ~/.diskgraph
  diskgraph search --scope <scope-id> --data-dir ~/.diskgraph --pattern '*.bin'
  diskgraph tree   --scope <scope-id> --data-dir ~/.diskgraph --depth 4

  # 4. connect an agent host to the MCP server
  codex mcp add diskgraph -- diskgraph-mcp --data-dir ~/.diskgraph --profile all

DATA

  Every command writes two SQLite files into --data-dir (default
  ./diskgraph-data): diskgraph.sqlite holds rebuildable graph snapshots,
  diskgraph-control.sqlite holds scope registrations, policies, jobs and
  operation history. Deleting the first is safe (rescan rebuilds it);
  deleting the second loses the registered scopes.

OUTPUT

  Without --json, output is human-oriented. With --json, stdout carries
  one JSON envelope and nothing else (logs go to stderr):

    diskgraph tree --scope <id> --data-dir ~/.diskgraph --depth 4 --json | jq .data.tree

EXIT CODES

  0  success                      5  stale plan / revision expired
  2  bad arguments                6  not supported in this build
  3  permission denied            7  budget exceeded
  4  not indexed / not found      8  partial completion
                                   9  conflict
                                  10  internal error

SCAN BEHAVIOUR

  The walk mirrors disktree's own flags: -a apparent size, -H no hidden,
  -x / -X filesystem boundary, -d depth, --no-dedup-hardlinks. Two
  snapshots are only comparable when scanned with the same options, and
  the snapshot records them. --max-nodes-per-scan and --max-staging-bytes
  are the budgets a walk stops at, for a named reason, instead of
  returning less data without saying so.

NOT SERVED BY THIS BUILD

  These catalog commands exist so a caller gets a clear unsupported
  answer rather than "unknown command" - but they do not act:

    duplicates  read  move  copy  trash  restore  purge
    plan  apply  operations

  Their engine and storage layers are implemented and tested
  (crates/diskgraph-engine, crates/diskgraph-ops); what is missing is
  the product wiring and the trusted review surface, so the CLI refuses
  them rather than pretending.
