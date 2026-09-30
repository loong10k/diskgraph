# diskgraph-disktree-core

The filesystem scanning library behind [DiskGraph](https://github.com/loong10k/diskgraph),
**vendored from [tobi/disktree](https://github.com/tobi/disktree)** at
revision `158f9cc2f0b332194a3ffc5acec47760c99146d8` (MIT, Copyright (c)
2026 Tobi Lütke).

## Install

DiskGraph depends on it as a path dependency; it is also published so the
scanner can be used on its own:

```toml
[dependencies]
diskgraph-disktree-core = "0.2"
```

## Why it is vendored

DiskGraph reuses disktree's scanner rather than reimplementing it. The
dependency was originally a git pin, which made every crate that touches
the scanner unpublishable to crates.io — the upstream library has never
been published there. Vendoring keeps the exact same source while turning
the dependency into a plain path dependency.

The source is **unmodified**; only the package name changed (to leave the
crates.io name `disktree-core` to the upstream author). Provenance, the
update procedure, and drift protection are documented in
[VENDORED.md](VENDORED.md) next to this file, and the
`pinned_upstream` test in `diskgraph-disktree` verifies this copy against
per-file digests of the pinned revision.

## What it provides

`ScanOptions`, the cancellable `ScanHandle`, the `Node` tree, size
measurement, and treemap layout. DiskGraph uses only the read-only
scanning surface; removal modules are not wired into its operation layer,
which has its own approval-gated path.

## License

MIT — the upstream license text is in `LICENSE` next to this file.
