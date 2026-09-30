# diskgraph-disktree

The read-only scanning bridge of [DiskGraph](https://github.com/loong10k/diskgraph):
it turns a directory walk into DiskGraph's lossless node model, preserving
locator bytes, file identity, and a separate own-bytes figure.

## Install

```toml
[dependencies]
diskgraph-disktree = "0.2"
```

## What it does

- `scan_native` — a v1-compatible projection of a walk
- `scan_native_v2` / `convert_tree` — the lossless form: raw locator
  bytes, `file_identity` (device/inode), and `direct_bytes` kept apart
  from the subtree total
- Hidden files, symlink loops, and hardlink deduplication follow the
  upstream walker's semantics exactly

## Parity

The walker's own option contract is mirrored one-for-one
(`apparent_size`, `follow_links`, `include_hidden`, `one_filesystem`,
`max_depth`, `dedup_hardlinks`), and `scan_options_parity.rs` in the
engine crate proves DiskGraph and the upstream walker report identical
byte totals for the same tree and the same options.

## License

MIT
