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
grant, a byte budget, and an identity contract: the object is re-checked
after the read, so a file that changed underneath is reported `Unstable`
rather than producing a digest of half of each version.

## License

MIT
