# CLI 实际编码连续撤权见证

沿用 SC-04、D24 与既有读取/编码连续撤权要求，不改变默认预算、50ms 终检窗口、结果字段或只读平台范围。

## 验收

- CLI tree、compare、changes、growth 和 HTML 的实际编码期间，原请求的真实主体、revision/scope 和撤权见证必须持续有效。
- 编码中撤销 MetadataRead 后恢复同一 grant，原请求仍拒绝输出；恢复后的新请求可以重新授权，不让原请求复活。
- 终检仍读取新鲜归属，禁止复用可能冻结旧 WAL 的连接、跳过归属或放宽期限；不能完成观察时仍失败关闭。
- 正常输出与 deadline partial 保持既有字段和预算，计划/HTML 不输出过期或未授权结果。编码可以因期限转换重做，回调不得自行发布文件或写出响应。

修复前 CLI snapshot_reply 在 Engine 查询返回后独立编码，再调用 finalize_revisions_read_until 重新绑定见证。当前修改复用原 Engine 请求内部的 finish 回调覆盖 tree/HTML、changes、growth、compare、带内容校验的 compare 与 plan 编码，不另开重复请求或新增预算。内容校验原地更新报告，避免编码重试时重复读取文件；只有原请求终态授权成功后才输出或写 HTML。

## 2026-10-09 实测记录

Windows 台式机仓库基线 abb87836，叠加当前修改，Rust 1.99.0 / x86_64-pc-windows-msvc。真实扫描 worker 按已有部署绑定验证摘要，默认测试并行和产品期限均未调整。原始日志保留于台式机临时目录 diskgraph-desktop-jetfdyj0。

- cli_encoding_withdrawal_red：旧 tree 实际返回 Ok，新增测试 0 通过 / 1 失败，确认行为漏洞。
- cli_encoding_withdrawal_green：tree 修复后 1 / 0。
- cli_encoding_withdrawal_expanded_red：tree/HTML 2 通过，compare/changes/growth/plan 4 项旧实现实际返回 Ok，确认同类漏洞。
- cli_encoding_withdrawal_verified_red：首次参数组合冲突，属于夹具失败，不计作行为红灯；保留日志。
- cli_encoding_withdrawal_verified_corrected_red：修正参数后，带内容校验的 compare 旧实现实际返回 Ok，0 / 1。
- cli_encoding_withdrawal_all_green：当前修改首次默认运行 2 / 5；5 项未到编码点即 BudgetExceeded，不能当作安全回归通过。
- cli_encoding_withdrawal_budget_diagnostic：相同源码与默认并行，额外启用诊断环境变量后 7 / 0；输入有区别，不替代上述失败或证明稳定性。
- cli_encoding_withdrawal_workspace：相同修改的全 workspace 默认并行，CLI 二进制单元测试 72 通过 / 12 失败，包含预算/调度失败；另有两项 Windows 完整退出状态测试缺少显式夹具绑定。no-fail-fast 继续运行至 Engine 原生 Git 测试，发生 probe deadline exceeded，清理发现未登记对象并保留原责任循环等待。确认原恢复 Drop 循环后显式停止该受控 Job；连接器终态 stopped / -1，不是完整 workspace 结果，也不证明产品有限退出。日志及原 source.diff 保留。
- windows_exit_status_bound：沿用仓库脚本和已安装 MSVC 构建真实 C 产物，显式绑定后直接运行此前编译的原测试二进制，两项 2 / 0（6.32 秒）。这确认此前两项是绑定缺失，不消除全量中的预算失败。
- cli_mcp_encoding_native_diagnostic：第一次定向选择器误用 --lib 未运行 CLI，保留该失败；纠正为 --bin diskgraph 后，当前相同源码、默认环境、默认并行的 7 项撤权恢复回归 7 / 0（4.39 秒）。不以定向通过替代此前全量失败。

macOS：CLI 测试编译、Engine/CLI Clippy、格式检查及 Engine 原连续撤权回归 6 / 0。未配置真实扫描宿主，不以这些结果替代 macOS CLI 原生集成验收。

状态：实现已进入回归，默认并行稳定性、完整 Windows workspace 及当前源码跨平台验收仍未完成；整体生产门禁保持开放。
