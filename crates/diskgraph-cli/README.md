# diskgraph-cli

The command line entry point of
[DiskGraph](https://github.com/loong10k/diskgraph) — scope management,
durable indexing, bounded queries, and a disktree-style JSON tree.

## Install

```bash
brew install loong10k/diskgraph/diskgraph      # macOS, Linux
npx -y diskgraph                                # any platform, no toolchain
cargo install diskgraph-cli                     # from source
```

Native Linux packages ship with each GitHub release:

```bash
sudo dpkg -i diskgraph_0.2.1_amd64.deb            # Debian, Ubuntu
sudo dnf install diskgraph-0.2.1-1.x86_64.rpm     # Fedora, RHEL
```

## A first run

```bash
diskgraph scope add --root ~/projects --data-dir ~/.diskgraph
diskgraph index --scope <scope-id> --data-dir ~/.diskgraph --wait
diskgraph tree  --scope <scope-id> --data-dir ~/.diskgraph --depth 3
```

Every command answers with a machine-readable envelope; `--json` keeps
stdout free of anything but JSON and sends logs to stderr.

## Scan options mirror disktree

| flag | meaning |
| :--- | :--- |
| `-a, --apparent-size` | measure apparent length, not allocated blocks |
| `-H, --no-hidden` | skip dotfiles (on by default) |
| `-x, -X` | stay on / cross filesystem boundaries |
| `-d, --depth N` | limit the walk depth; totals below it stay unknown |
| `--no-dedup-hardlinks` | count a hardlinked file once per link |
| `--max-nodes-per-scan`, `--max-staging-bytes` | scan budgets; a reached limit stops for a named reason |

Two snapshots are only comparable when they were scanned with the same
options, and the snapshot records them.

## Exit codes

`2` bad arguments · `3` permission denied · `4` not indexed or not found ·
`5` stale plan or revision expired · `6` unsupported · `7` budget
exceeded · `8` partial · `9` conflict · `10` internal.

## License

MIT
