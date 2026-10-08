# MCP 历史响应编码连续撤权

沿用 SC-04/D24 的真实主体、权限交集与请求连续撤权契约，不调整预算、50ms 终检窗口或 wire 字段。

- changes/growth 在实际 envelope 编码期间撤销任一侧 MetadataRead grant，即使立即恢复同一 grant，原请求仍拒绝输出。
- 新请求可以取得恢复后的权限；无关范围权限不受影响。
- 使用真实 bearer、Origin、TCP socket 与隔离数据库，合法导入版本只用于查询安全契约，不作为原生扫描验收证据；现有实际扫描夹具回归保留。
- 编码必须位于原 Engine 双侧请求的撤权见证生命周期内，不以编码后独立查询重建见证。

## 2026-10-09 实施与验证

macOS 上合法导入查询夹具、真实 bearer/Origin/TCP socket 的两项新增回归先记录 0 / 2：旧 changes/growth 在撤权恢复后实际返回成功结构化数据。当前修改将 envelope 编码移入 Engine 原双侧请求的 finish 回调，移除生产路径的编码后独立见证捕获；文本与结构化结果仅在 Engine 终检成功后发布。对已准入数据只复制一次构造 envelope，不再为编码额外复制整个 envelope。

原契约区分原生精确撤权通知与无原生通知平台：前者拒权，后者发生授权代次变化时保守返回 conflict。第一次修复后测试错误地要求两者都返回 permission_denied，0 / 2；实际已经返回 conflict 且无数据，保留该测试断言失败记录。按原能力契约分别断言准确错误码后，最终回归 2 / 0，每项覆盖两侧，并确认恢复后的新请求可以成功。

本机受影响回归：filesystem_deadline_tests 8 / 0；request_deadline_tests 22 / 0；最终 MCP Clippy 通过。此夹具不证明原生扫描。Windows 正在运行的完整任务使用此前 CLI 修复源码，不包含本次 MCP 修改，不将其结果记为 MCP 新源码验收。

Windows 已结构化更新上述六个 MCP 文件，实际部署绑定不变。默认并行 MCP 全 lib 223 项：219 通过 / 4 失败 / 0 忽略，45.62 秒（含构建整次 142.359 秒）。新增 growth 撤权恢复回归通过；新增 history 已拒绝原请求，但恢复后的新请求预算耗尽，整体仍失败。其余三项分别是捕获权限撤销期待 PermissionDenied 实得 BudgetExceeded、过期期限合法部分诊断 BudgetExceeded、损坏数据期待 internal_error 实得 budget_exceeded。没有通过修改断言、期限或串行化消除这些失败。

同批 CLI 定向选择器误用 --lib（该 package 仅有 binary），未执行 CLI 测试；该失败单独保留，后续使用真实 --bin diskgraph 纠正。Windows 原始日志、receipt 与 source.diff 位于台式机临时目录 cli_mcp_encoding_native_final，变更摘要 dafeb2fd0b39c2f408a2965336fe5c6794b6c4f296ad767273ad9852a3b74edd。后续 MCP 定向启用诊断，只用于定位预算阶段，不替代此次默认全量失败。

后续 cli_mcp_encoding_native_diagnostic：纠正选择器的 CLI 默认环境/默认并行 7 / 0（4.39 秒）；MCP 开启诊断的撤权恢复 2 / 0（1.60 秒）、filesystem_deadline_tests 8 / 0（1.42 秒）。本次定向未产生预算阶段诊断，不据此推断此前失败原因或撤销默认全量反例。17 个修改源文件摘要与本机一致；记录在 docs/benchmarks/encoding_withdrawal_2026_10_09/receipt.json。

状态：历史两项生产路径已修改并有本机及 Windows 定向安全回归证据；关系类响应编码连续性仍需检查修复，Windows 默认全量稳定性及完整跨平台验收未完成。整体生产门禁保持开放。
