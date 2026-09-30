# Vendored: disktree-core

This directory is an **unmodified copy** of the `disktree-core` crate from
[tobi/disktree](https://github.com/tobi/disktree) at revision
`158f9cc2f0b332194a3ffc5acec47760c99146d8`, licensed MIT
(Copyright (c) 2026 Tobi Lütke; the license text is in `LICENSE`).

## Why it is vendored

DiskGraph reuses disktree's scanner rather than reimplementing it. The
dependency was originally a git pin, which made the whole chain exact and
reproducible — but it also made every crate that touches the scanner
unpublishable to crates.io, because crates.io requires registry
dependencies and the upstream library has never been published there
(the `disktree` package on crates.io is a different project's TUI
application, not this library).

Vendoring keeps the exact same source while turning the dependency into a
plain path dependency, so all DiskGraph crates become publishable. The
source is **not** edited here: fixes belong upstream, and are brought in by
re-copying the upstream revision.

## How drift is caught

`crates/diskgraph-disktree/tests/pinned_upstream.rs` records the
per-file digests of the pinned upstream revision and fails if this copy
diverges. To update the pin:

1. Change `UPSTREAM_REV` in that test to the new upstream revision.
2. Re-copy `crates/disktree-core` from the new revision's checkout.
3. Regenerate the digests in the test and update this file's revision line.
4. Run the workspace gates; the parity tests (`scan_options_parity.rs`)
   must still show byte-identical walks against the pinned options.

## What DiskGraph uses

Only the read-only scanning surface: `ScanOptions`, `ScanHandle`,
`Node`, `Metric`, and the size/coverage helpers. The removal modules
(`removal.rs`) and any application code are deliberately not wired into
DiskGraph's own operation layer, which has its own approval-gated path.
