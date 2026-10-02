import Foundation
import SQLite3

func payload(_ text: String) -> Any {
    guard let data = text.data(using: .utf8),
          let envelope = try? JSONSerialization.jsonObject(with: data) as? [String:Any],
          envelope["ok"] as? Bool == true,
          let value = envelope["data"] else { fatalError("FFI failed: \(text)") }
    return value
}
let root = CommandLine.arguments[1]
let db = CommandLine.arguments[2]
// 同一进程实际读写宿主 SQLite；这里只证明系统 SQLite，不声明 GRDB 集成。
var host: OpaquePointer?
precondition(sqlite3_open((db+".host").cString(using:.utf8),&host)==SQLITE_OK)
precondition(sqlite3_exec(host,"CREATE TABLE host_value(value INTEGER); INSERT INTO host_value VALUES(42);",nil,nil,nil)==SQLITE_OK)
let service = try NativeService(databasePath:db)
let handle = try service.spawnScan(rootPath:root)
_ = payload(handle.progressJson())
let deadline = Date().addingTimeInterval(30)
var result: String?
while result == nil {
    result = handle.pollResultJson()
    if result == nil { precondition(Date()<deadline); Thread.sleep(forTimeInterval:0.001) }
}
let scan = payload(result!) as! [String:Any]
let snapshot = scan["snapshot_id"] as! String
precondition((scan["node_count"] as! NSNumber).intValue>=3)
for _ in 0..<10 {
    let node = payload(service.nodeJson(snapshotId:snapshot,nodeId:1)) as! [String:Any]
    precondition((node["id"] as! NSNumber).intValue==1)
    precondition(sqlite3_exec(host,"UPDATE host_value SET value=value+1;",nil,nil,nil)==SQLITE_OK)
}
let children = payload(service.childrenJson(snapshotId:snapshot,parentId:1,offset:0,limit:1)) as! [String:Any]
precondition((children["items"] as! [Any]).count==1)
precondition(!(children["next_offset"] is NSNull))
_ = payload(service.candidatesJson(snapshotId:snapshot,targetBytes:1))
let v1scan = payload(scanNativeJson(databasePath:db,rootPath:root)) as! [String:Any]
let top = payload(topJson(databasePath:db,snapshotId:v1scan["snapshot_id"] as! String,parentId:1,limit:10)) as! [Any]
precondition(top.count>=2)
let latest = payload(latestNativeSnapshotJson(databasePath:db,rootPath:root)) as! [String:Any]
precondition(latest["snapshot_id"] as! String == v1scan["snapshot_id"] as! String)
_ = payload(capabilitiesJson())
service.shutdown()
precondition(service.nodeJson(snapshotId:snapshot,nodeId:1).contains("\"ok\":false"))
precondition(sqlite3_close(host)==SQLITE_OK)
print("Swift session, paging, poll, v1 and host SQLite CRUD passed")
