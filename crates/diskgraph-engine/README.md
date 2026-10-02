# diskgraph-engine

Job orchestration and the authorized service layer of
[DiskGraph](https://github.com/loong10k/diskgraph): scope registration,
durable index jobs with leases, staging with atomic publication, capacity
watermarks, and the tree/read narrow paths.

## Install

```toml
[dependencies]
diskgraph-engine = "0.2"
```

## What it enforces

- **Scopes** are registered against a canonical root and revocable; a
  revoked scope yields no plan and accepts no job.
- **Jobs** are leased with owner fencing: a queued or running job merges
  rather than duplicating, and a terminal job is never re-claimed.
- **Budgets** stop a walk for a named reason (`NodeLimit`, `StagingLimit`,
  `TimeLimit`, `Cancelled`) instead of returning less data silently, and a
  stopped walk publishes nothing.
- **Capacity** watermarks refuse new work past a threshold and never
  delete anything to make room.
- **Cancellation** is observed between observation boundaries, so a
  cancelled scan leaves no half-published revision.

## Bounded content reads

`read_bounded` and `digest_bounded` run under a separate `content:read`
grant, a byte budget, and an identity contract: observed identity/version
changes report `Unstable` and void the digest. This is not an atomic snapshot.

Windows content acquisition uses a local absolute drive path and one
component at a time relative to retained directory handles. It requests
attributes before data, exposes placeholder attributes on the calling
thread, and refuses reparse/offline/recall objects before reading. The
read retains a data handle denying ordinary write/delete sharing and checks the full 128-bit
file ID, volume ID, length and native write/change timestamps. ADS,
parent traversal, UNC/device paths and unknown volume/identity support
are refused. The registered root's component spelling is required;
lexical checks do not guess case/alias equivalence. Attribute acquisition
does not freeze new writers: changes are rejected as conflict before
data access. Native timestamp units do not guarantee filesystem precision.

Sharing excludes ordinary write/delete handles, not every writable
mapping, kernel or filter mutation. Version checks are not an atomic
content snapshot. Windows `FILE_OPEN_NO_RECALL` constrains opening;
real provider no-download acceptance remains separate, as does Linux
placeholder protection. The optional thread API (Windows 10 1709+) is
dynamically resolved and missing support refuses inspection. Deadline and
cancellation checks are cooperative and cannot preempt synchronous native
opens or reads. Native evidence covers CI NTFS fixtures, not every local
filesystem. See the [full-platform acceptance record](../../docs/production-readiness-full-platform-2026-10-02.md).

## License

MIT
