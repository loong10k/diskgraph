import Foundation
import SQLite3

// 9.3: the host's own SQLite (system libsqlite3) coexists in this process
// with the static library's bundled copy.
print("host sqlite:", String(cString: sqlite3_libversion()))

let root = CommandLine.arguments[1]
let db = CommandLine.arguments[2]

// 9.2: the handle returns at once; the UI thread could poll right here.
let handle = spawnScanJson(databasePath: db, rootPath: root)
let progress = handle.progressJson()
print("immediate progress:", progress)

let result = handle.resultJson()
guard let data = result.data(using: .utf8),
      let envelope = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
      envelope["ok"] as? Bool == true else {
    print("FAILED:", result)
    exit(1)
}
let payload = envelope["data"] as? [String: Any]
print("scan ok: nodes =", payload?["node_count"] ?? "?")

// The synchronous v1 surface keeps its own snapshot store: scan through
// it, then query through it (v1 compatibility, PF-01).
let scanned = scanNativeJson(databasePath: db, rootPath: root)
guard let scanData = scanned.data(using: .utf8),
      let scanEnvelope = try? JSONSerialization.jsonObject(with: scanData) as? [String: Any],
      scanEnvelope["ok"] as? Bool == true,
      let scanPayload = scanEnvelope["data"] as? [String: Any],
      let snapshotId = scanPayload["snapshot_id"] as? String else {
    print("FAILED v1 scan:", scanned)
    exit(1)
}
let top = topJson(databasePath: db, snapshotId: snapshotId, parentId: 1, limit: 10)
print("v1 top:", top.contains("\"ok\":true"))
let latest = latestNativeSnapshotJson(databasePath: db, rootPath: root)
print("v1 latest:", latest.contains("snapshot_id"))

// The latest-rent lookup and the capabilities report close the loop.
let capabilities = capabilitiesJson()
print("capabilities:", capabilities.contains("read_only_queries"))
