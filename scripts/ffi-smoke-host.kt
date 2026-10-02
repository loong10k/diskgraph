import uniffi.diskgraph_ffi.NativeService
import uniffi.diskgraph_ffi.capabilitiesJson
import uniffi.diskgraph_ffi.topJson
import uniffi.diskgraph_ffi.scanNativeJson

fun good(text: String): String { check(text.contains("\"ok\":true")) { text }; return text }
fun snapshot(text: String): String = Regex("\"snapshot_id\":\"([^\"]+)\"").find(text)!!.groupValues[1]
fun main(args: Array<String>) {
    fun path(index: Int): String = if (args.getOrNull(3)=="utf8-base64")
        String(java.util.Base64.getDecoder().decode(args[index]), Charsets.UTF_8) else args[index]
    val rootPath = path(0)
    val databasePath = path(1)
    val service = NativeService(databasePath)
    val handle = service.spawnScan(rootPath)
    good(handle.progressJson())
    val deadline = System.nanoTime()+30_000_000_000L
    var result = handle.pollResultJson()
    while (result == null) { check(System.nanoTime()<deadline); Thread.sleep(1); result=handle.pollResultJson() }
    val id = snapshot(good(result))
    args.getOrNull(2)?.let { expected ->
        check(Regex("\"node_count\":$expected(?=[,}])").containsMatchIn(result)) { result }
        check(good(service.childrenJson(id,1u,0u,10u)).contains("文件-é-ß.txt"))
    }
    check(good(service.nodeJson(id,1u)).contains("\"id\":1"))
    val children=good(service.childrenJson(id,1u,0u,1u))
    check(children.contains("\"items\":["))
    check(!children.contains("\"next_offset\":null"))
    good(service.candidatesJson(id,1u))
    val legacy=snapshot(good(scanNativeJson(databasePath,rootPath)))
    check(good(topJson(databasePath,legacy,1u,10u)).contains("blob.bin"))
    good(capabilitiesJson())
    service.shutdown()
    check(service.nodeJson(id,1u).contains("\"ok\":false"))
    handle.close()
    service.close()
    val reopened = NativeService(databasePath)
    check(good(reopened.nodeJson(id,1u)).contains("\"id\":1"))
    reopened.shutdown()
    reopened.close()
    println("Kotlin session, paging, poll, v1, release and reopen passed")
}
