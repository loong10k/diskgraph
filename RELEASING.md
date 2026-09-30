# Releasing DiskGraph

One document for every distribution channel, in dependency order: the
GitHub release is the foundation; every other channel points at its
assets. Credentials live with the maintainer — nothing in this repo
contains a token.

## 0. Preconditions

| Need | Used by | Status |
| :--- | :--- | :--- |
| GitHub push rights on `loong10k/diskgraph` | release workflow, all channels | required |
| crates.io API token | crates.io publish | required for the two publishable crates |
| npm publish token (`_authToken`) | npm thin-installer | required |
| Apple Developer ID + notarization | macOS Gatekeeper | **not configured** — Gatekeeper warns |
| Windows Authenticode certificate | SmartScreen | **not configured** — SmartScreen warns |

Unsigned artifacts are stated as such in the release notes, the tap README,
and this file. This is a known, recorded gap, not a silent one.

## 1. The release chain (foundation)

Tag and push. One workflow runs the whole chain in order — valid tag,
binaries for five targets, a packaging dry run, crates.io, npm, and only
then the GitHub Release that every installer resolves against. A registry
failure therefore leaves no release pointing at a crate that never
landed:

```bash
git tag v0.1.0 && git push origin v0.1.0
```

Targets: `aarch64-apple-darwin`, `x86_64-apple-darwin`,
`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
`x86_64-pc-windows-msvc`. The release job re-verifies every archive
against the digest the build job computed and refuses to publish on any
mismatch, then smoke-checks that the archive contains both binaries.

Re-running for an existing tag: `gh workflow run release.yml -f tag=v0.1.0`.
Registry publishes are idempotent (an already-uploaded version counts as
success), and release assets are re-uploaded with `--clobber`. Set
`-f registries=false` to rebuild and refresh the release assets without
touching the registries.

## 2. Homebrew tap

`loong10k/homebrew-diskgraph` holds `Formula/diskgraph.rb` with the
version and per-architecture `sha256` pinned. After a release, update those
four values and push:

```bash
scripts/sync-release-digests.sh v0.1.0   # fills the formula in place
```

The tag must be exactly `vX.Y.Z`; the chain refuses anything else.

## 3. npm (thin installer)

The `npm/` directory is the package; it ships no binaries, only a
launcher plus an installer that downloads the platform archive, verifies
its SHA-256, and unpacks `diskgraph` and `diskgraph-mcp`. Bump
`VERSION` in `npm/lib/platform.js` together with the release, then:

```bash
cd npm && npm publish --access public
```

`npx -y diskgraph --version` installs and runs without a toolchain.

## 4. crates.io

Only the pure-model crates are publishable: `diskgraph-core` has no git
dependency, and `diskgraph-store` depends solely on it. Everything that
touches the scanner depends on `disktree-core`, which is pinned to a git
revision (upstream is not on crates.io, and the pin is exactly what makes
a scan reproducible), so those seven crates carry `publish = false` with
the reason inline. Publishing order matters:

```bash
cargo publish -p diskgraph-core     # first: everything else may depend on it
cargo publish -p diskgraph-store    # second, once core is on the registry
```

`cargo publish --dry-run` verifies packaging and a clean standalone
build before anything is uploaded. Note: `cargo install diskgraph-cli` is
therefore **not** a channel; source installs are
`cargo install --path crates/diskgraph-cli` from a checkout, and binary
users use the release, tap, npm, winget, or scoop.

## 5. Windows managers

`packaging/winget/loong10k.DiskGraph.yaml` and
`packaging/scoop/diskgraph.json` are kept in-repo so each release can open
(or update) a pull request against the upstream manifest repositories.
Submitting them requires a winget-pkgs / Scoop-Shells PR, which is a
maintainer action with those repositories' own review rules.

## Channel matrix

| Channel | Command | Ships |
| :--- | :--- | :--- |
| npm | `npx -y diskgraph --version` | prebuilt, digest-verified |
| Homebrew | `brew install loong10k/diskgraph/diskgraph` | prebuilt, digest-verified |
| GitHub Releases | direct download | prebuilt + SHA256SUMS |
| winget | `winget install loong10k.DiskGraph` | prebuilt |
| scoop | `scoop install diskgraph` | prebuilt |
| crates.io | `cargo add diskgraph-core` | source (library crates) |
| source | `cargo install --path crates/diskgraph-cli` | source build |

Every binary channel resolves to the same release artifacts, and every
installer verifies the published digest before unpacking.
