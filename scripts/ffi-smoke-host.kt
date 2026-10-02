import uniffi.diskgraph_ffi.NativeService
import uniffi.diskgraph_ffi.capabilitiesJson
import uniffi.diskgraph_ffi.topJson
import uniffi.diskgraph_ffi.scanNativeJson

fun good(text: String): String { check(text.contains("\"ok\":true")) { text }; return text }
fun snapshot(text: String): String = Regex("\"snapshot_id\":\"([^\"]+)\"").find(text)!!.groupValues[1]
fun main(args: Array<String>) {
    val service = NativeService(args[1])
    val handle = service.spawnScan(args[0])
    good(handle.progressJson())
    val deadline = System.nanoTime()+30_000_000_000L
    var result = handle.pollResultJson()
    while (result == null) { check(System.nanoTime()<deadline); Thread.sleep(1); result=handle.pollResultJson() }
    val id = snapshot(good(result))
    check(good(service.nodeJson(id,1u)).contains("\"id\":1"))
    val children=good(service.childrenJson(id,1u,0u,1u))
    check(children.contains("\"items\":["))
    check(!children.contains("\"next_offset\":null"))
    good(service.candidatesJson(id,1u))
    val legacy=snapshot(good(scanNativeJson(args[1],args[0])))
    check(good(topJson(args[1],legacy,1u,10u)).contains("blob.bin"))
    good(capabilitiesJson())
    service.shutdown()
    check(service.nodeJson(id,1u).contains("\"ok\":false"))
    handle.close()
    println("Kotlin session, paging, poll and v1 passed")
}
