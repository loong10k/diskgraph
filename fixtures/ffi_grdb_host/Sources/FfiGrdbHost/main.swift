import DiskGraphNative
import Darwin
import Foundation
import GRDB

func verify(_ condition: Bool, _ message: String = "fixture assertion failed", file: StaticString = #fileID, line: UInt = #line) {
    if !condition {
        FileHandle.standardError.write(Data("\(file):\(line): \(message)\n".utf8))
        exit(1)
    }
}

// 并发测试汇集错误；不得把后台失败当成宿主成功。
final class Failures: @unchecked Sendable {
    private let lock = NSLock()
    private var messages: [String] = []
    func capture(_ action: () throws -> Void) {
        do { try action() } catch {
            lock.lock()
            messages.append(String(describing: error))
            lock.unlock()
        }
    }
    func assertEmpty() {
        lock.lock()
        defer { lock.unlock() }
        verify(messages.isEmpty, "Concurrent host errors: \(messages)")
    }
}

func payload(_ text: String) -> [String: Any] {
    let envelope = try! JSONSerialization.jsonObject(with: Data(text.utf8)) as! [String: Any]
    verify(envelope["ok"] as? Bool == true, "FFI failed: \(text)")
    return envelope["data"] as! [String: Any]
}

// macOS 上直接检查图数据库相关描述符，避免把“两连接同时打开”误称释放。
func graphDescriptors(_ path: String) throws -> Int {
    guard let resolved = path.withCString({ realpath($0, nil) }) else {
        throw NSError(domain: NSPOSIXErrorDomain, code: Int(errno))
    }
    defer { free(resolved) }
    let canonical = String(cString: resolved)
    var count = 0
    for entry in try FileManager.default.contentsOfDirectory(atPath: "/dev/fd") {
        guard let descriptor = Int32(entry) else { continue }
        var buffer = [CChar](repeating: 0, count: Int(MAXPATHLEN))
        let status = buffer.withUnsafeMutableBufferPointer { pointer in
            fcntl(descriptor, F_GETPATH, pointer.baseAddress!)
        }
        if status != 0 { continue }
        let observed = String(decoding: buffer.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }, as: UTF8.self)
        if observed == canonical || observed == canonical + "-wal" || observed == canonical + "-shm" {
            count += 1
        }
    }
    return count
}

let root = CommandLine.arguments[1]
let directory = CommandLine.arguments[2]
let hostPath = directory + "/host.sqlite"
let graphPath = directory + "/graph.sqlite"
var pool: DatabasePool? = try DatabasePool(path: hostPath)
try pool!.write { db in
    try db.execute(sql: "CREATE TABLE host_event(id INTEGER PRIMARY KEY, value TEXT NOT NULL)")
}
var service: NativeService? = try NativeService(databasePath: graphPath)
let failures = Failures()
let workers = DispatchGroup()
let hostPool = pool!
// GRDB 管理自己的 WAL 连接池；四个后台写入者与一个读取者同时运行。
for writer in 0..<4 {
    workers.enter()
    DispatchQueue.global().async {
        defer { workers.leave() }
        failures.capture {
            for item in 0..<100 {
                try hostPool.write { db in
                    try db.execute(sql: "INSERT INTO host_event(id,value) VALUES (?,?)",
                                   arguments: [writer * 100 + item, "initial"])
                    try db.execute(sql: "INSERT INTO host_event(id,value) VALUES (?,?)",
                                   arguments: [10000 + writer * 100 + item, "temporary"])
                    try db.execute(sql: "UPDATE host_event SET value=? WHERE id=?",
                                   arguments: ["host-\(writer)-\(item)", writer * 100 + item])
                    try db.execute(sql: "DELETE FROM host_event WHERE id=?",
                                   arguments: [10000 + writer * 100 + item])
                }
                Thread.sleep(forTimeInterval: 0.001)
            }
        }
    }
}
workers.enter()
DispatchQueue.global().async {
    defer { workers.leave() }
    failures.capture {
        for _ in 0..<100 {
            let count = try hostPool.read { db in try Int.fetchOne(db, sql: "SELECT COUNT(*) FROM host_event")! }
            verify((0...400).contains(count))
            Thread.sleep(forTimeInterval: 0.001)
        }
    }
}

// Rust 扫描与宿主业务事务共存；轮询和查询在此命令行后台验收宿主中执行。
var handle: JobHandle? = try service!.spawnScan(rootPath: root)
let deadline = Date().addingTimeInterval(60)
var result = handle!.pollResultJson()
while result == nil {
    verify(Date() < deadline, "Native scan timed out")
    _ = payload(handle!.progressJson())
    Thread.sleep(forTimeInterval: 0.001)
    result = handle!.pollResultJson()
}
let scan = payload(result!)
verify((scan["node_count"] as! NSNumber).intValue == 2001)
let snapshot = scan["snapshot_id"] as! String
for _ in 0..<50 {
    let node = payload(service!.nodeJson(snapshotId: snapshot, nodeId: 1))
    verify((node["id"] as! NSNumber).intValue == 1)
    let children = payload(service!.childrenJson(snapshotId: snapshot, parentId: 1, offset: 0, limit: 10))
    verify((children["items"] as! [Any]).count == 10)
}
verify(workers.wait(timeout: .now() + 60) == .success, "GRDB workers timed out")
failures.assertEmpty()
let count = try hostPool.read { db in try Int.fetchOne(db, sql: "SELECT COUNT(*) FROM host_event")! }
verify(count == 400)
let updatedCount = try hostPool.read { db in try Int.fetchOne(db, sql: "SELECT COUNT(*) FROM host_event WHERE value LIKE 'host-%'")! }
verify(updatedCount == 400, "updates and removal of temporary rows must persist")
try hostPool.close()
pool = nil
let openedDescriptorCount = try graphDescriptors(graphPath)
verify(openedDescriptorCount > 0, "the first Rust session must own an open graph descriptor")
service!.shutdown()
verify(service!.nodeJson(snapshotId: snapshot, nodeId: 1).contains("\"ok\":false"))

// 两者均持有 Rust Engine；逻辑 shutdown 不等于物理连接释放。
handle = nil
service = nil
let releaseDeadline = Date().addingTimeInterval(5)
while try graphDescriptors(graphPath) != 0 {
    verify(Date() < releaseDeadline, "Rust graph descriptors survived final host reference release")
    Thread.sleep(forTimeInterval: 0.001)
}
// 释放包装引用后重开两个独立存储，验证生命周期和持久化。
let reopenedPool = try DatabasePool(path: hostPath)
let persistedCount = try reopenedPool.read { db in try Int.fetchOne(db, sql: "SELECT COUNT(*) FROM host_event")! }
verify(persistedCount == 400)
let reopenedService = try NativeService(databasePath: graphPath)
let persistedNode = payload(reopenedService.nodeJson(snapshotId: snapshot, nodeId: 1))
verify((persistedNode["id"] as! NSNumber).intValue == 1)
reopenedService.shutdown()
try reopenedPool.close()
print("GRDB 7.11.1 / Rust dynamic FFI coexistence passed: 400 host rows, 2001 graph nodes, concurrent CRUD, close and reopen")
