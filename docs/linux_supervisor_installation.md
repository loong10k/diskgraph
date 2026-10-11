# Linux fixed supervisor installation artifacts

The broker now routes JSON `doctor` and fixed read-only metadata queries: scope list/show, status, snapshot listing, tree without HTML, node, children, top, search, explore, growth, changes, explain, related, impact and candidates. Native qualification of the added queries is pending CI. Scanning, maintenance, content and file output commands, MCP forwarding, macOS and Windows services remain under implementation. The existing direct read-only entry points retain their current contracts.

On an isolated Linux host, a trusted root installer prepares a fresh prefix from an independently verified archive containing `diskgraph`, `diskgraph-mcp`, `diskgraph-scan-worker`, `diskgraph-supervisor` and the existing worker manifest:

```sh
python3 scripts/prepare_linux_supervisor_installation.py \
  --archive /protected/diskgraph-x86_64-unknown-linux-gnu.tar.gz \
  --expected-sha256 VERIFIED_ARCHIVE_SHA256 \
  --prefix /var/lib/diskgraph-isolated \
  --service-uid 23111 --service-gid 23111 --frontend-uid 23112 --with-broker
```

The frontend and service identities must differ. This first broker mode supports exactly one frontend UID. The prefix must be new, its ancestry root owned and protected, and the resulting socket path at most 100 UTF-8 bytes. Existing installations and ACTIVE records are never replaced.

The preparation writes fixed executable images, a root owned mode-0600 `supervisor-config.json`, dedicated `state/data`, original bootstrap/service slots, and an unenabled `diskgraph-supervisor.service`. It never invokes `systemctl`. The root broker authenticates the frontend; the business role runs as the dedicated service UID.

The frontend uses `diskgraph --json --supervisor-socket /var/lib/diskgraph-isolated/front.sock doctor`. Its original 30-second budget includes connection, authenticated frames, output and terminal EOF. Client completion does not release recovery capacity: the original supervisor owner must retire and both original slots must become CLEAN.

The generated unit disables automatic restart. Unexpected failure retains ACTIVE for explicit recovery; restarting or removing a socket does not prove resource retirement. The isolated CI gate checks the real prepared images and unit syntax, then runs that generated unit under a random exclusive name in the temporary Linux runner. It observes both original slot FDs as CLEAN before stopping that unit. Failure to stop retains the unit, installation and original records. Static syntax checks alone do not satisfy this runtime gate. This new runtime gate is awaiting native qualification. No desktop service has been installed, and production activation remains pending native service lifecycle qualification.

For example: `diskgraph --json --supervisor-socket /var/lib/diskgraph-isolated/front.sock scope list`. Only declared command arguments are forwarded; the installation fixes the principal and data directory. Unknown arguments, global overrides and requests exceeding 4KiB are rejected before birth. Results retain the original JSON and exit code, with output capped at 1MiB. Engine still checks resource permissions.

`scope show` uses the authorized metadata entry point. Missing MetadataRead permission, scope revocation, the original request deadline or a terminal capability callback exceeding 50ms prevents delivery. Root display is released only after live checks following encoding. The trusted internal `Engine::scope` API remains available but is no longer used for this public read. Native qualification of the added cases is pending CI.

The CLI node, children, top, explore and search handlers now share the original dispatch deadline and an independently authorized reader. The requested scope must match the revision's stored ownership; encoded data is released only after live terminal authorization. Search cursors bind to the actual dispatch principal. Isolated metadata tests cover these handlers; current-source native service qualification is still required. Filtered children and search retain the existing page decoding limits; this change does not establish a strict raw-byte or RSS bound for every query.

CLI scan-job status now intersects OperationView capability with the live grant for the job's actual scope. Status and snapshot listing preserve the original dispatch deadline and prepare bounded responses before terminal checks. Their original withdrawal witness survives response encoding, so revoking and restoring a grant cannot authorize an in-flight response; an unavailable native watch yields Conflict on generation change. Native qualification remains separate from isolated metadata regressions.

Filtered CLI children pages (including unknown sizes) and search now admit the actual SQLite row fields before owning strings or decoding legacy payloads. Oversized raw pages fail with budget_exceeded even when their final response would be small; no partial cursor is issued. Explore counts its parent toward max_nodes. Isolated 200k-row release checks confirm page-sized decoding and preserved directory indexes, but do not establish a native scan, RSS bound or latency SLA.

The generated systemd unit bounds the root broker to `CAP_SETUID CAP_SETGID CAP_KILL` and explicitly preserves only `CAP_SETUID` in its ambient set. The birth role clears ambient and effective capabilities before executing the business image. Native qualification observes the actual MainPID capability masks (`CapEff` and `CapBnd` = `0xe0`, `CapAmb` = `0x80`) before the original frontend request, then verifies original-owner retirement and CLEAN records. This fixes a reproduced missing-SETUID startup refusal; isolated C tests alone do not qualify the generated service. Current-source service qualification remains pending CI.
