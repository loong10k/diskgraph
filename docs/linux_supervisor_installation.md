# Linux fixed supervisor installation artifacts

The current broker forwards JSON `doctor` only. Other CLI commands, MCP forwarding, macOS and Windows services remain under implementation. The existing direct read-only entry points retain their current contracts.

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

The generated unit disables automatic restart. Unexpected failure retains ACTIVE for explicit recovery; restarting or removing a socket does not prove resource retirement. Isolated CI verifies the real prepared images and the actual unit syntax. Static unit verification is distinct from running a systemd service. No desktop service has been installed, and production activation remains pending native service lifecycle qualification.
