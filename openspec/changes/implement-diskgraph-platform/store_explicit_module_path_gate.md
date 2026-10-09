# Store 结构门禁的显式模块路由修复

CI `37876549881` 的 Linux stable/MSRV/ARM64 和 macOS stable/MSRV 实际在 `source_layout.rs:117` 失败：测试递归只采用默认模块目录，未处理 `policy_store.rs` 的合法 `#[path = "live_identity_permission.rs"]`。这些作业的实际失败记录不能记作平台通过；这不是该授权模块无法编译。

增加真实临时 Rust 文件夹具：显式模块必须访问真实来源并发现未文档类型，默认位置存在诱饵也不能覆盖显式路径；普通非入口文件的相对路径以声明所在文件为基准。原测试真实 RED 0/1（NotFound），解析属性后 GREEN 3/0，含既有整个 Store 结构门禁。未跳过新授权模块或减少类型/文档/wildcard/占位检查。Windows 第一轮原生 Store 全目标中本门禁实际3/0，其余失败另记在暂存 checkpoint 合同，不借本门禁通过声称全包成功。

本修复仅改变门禁的来源定位，不改变 Rust 编译路由或生产授权逻辑。同 SHA 全平台 CI 仍需执行。
