import uniffi.diskgraph_ffi.*

fun main(args: Array<String>) {
    val root = args[0]
    val db = args[1]
    // 9.2 on the Kotlin side: the handle returns at once.
    val handle = spawnScanJson(databasePath = db, rootPath = root)
    println("immediate: " + handle.progressJson())
    val result = handle.resultJson()
    println("joined: " + if (result.contains("\"ok\":true")) "ok" else result)
    // The v1 read-only surface still answers.
    val capabilities = capabilitiesJson()
    println("capabilities ok: " + capabilities.contains("\"ok\":true"))
    if (!result.contains("\"ok\":true")) kotlin.system.exitProcess(1)
}
