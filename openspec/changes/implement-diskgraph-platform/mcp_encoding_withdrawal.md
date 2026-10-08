# MCP 历史响应编码连续撤权

沿用 SC-04/D24 的真实主体、权限交集与请求连续撤权契约，不调整预算、50ms 终检窗口或 wire 字段。

- changes/growth 在实际 envelope 编码期间撤销任一侧 MetadataRead grant，即使立即恢复同一 grant，原请求仍拒绝输出。
- 新请求可以取得恢复后的权限；无关范围权限不受影响。
- 使用真实 bearer、Origin、TCP socket 与隔离数据库，合法导入版本只用于查询安全契约，不作为原生扫描验收证据；现有实际扫描夹具回归保留。
- 编码必须位于原 Engine 双侧请求的撤权见证生命周期内，不以编码后独立查询重建见证。

关系类同一契约覆盖 related、explain、impact、candidates：实际 envelope 编码期间撤销并恢复当前 scope 的 MetadataRead，原请求须拒绝，恢复后的新请求可成功；准确拒绝码继续按原生精确见证/保守代次冲突区分。保留原版本归属、原期限、累计读取账本、响应字节限制及截断字段。显式单实体采集批次仅用于查询安全夹具，不宣称原生扫描或采集能力通过。

## 2026-10-09 实施与验证

macOS 上合法导入查询夹具、真实 bearer/Origin/TCP socket 的两项新增回归先记录 0 / 2：旧 changes/growth 在撤权恢复后实际返回成功结构化数据。当前修改将 envelope 编码移入 Engine 原双侧请求的 finish 回调，移除生产路径的编码后独立见证捕获；文本与结构化结果仅在 Engine 终检成功后发布。对已准入数据只复制一次构造 envelope，不再为编码额外复制整个 envelope。

原契约区分原生精确撤权通知与无原生通知平台：前者拒权，后者发生授权代次变化时保守返回 conflict。第一次修复后测试错误地要求两者都返回 permission_denied，0 / 2；实际已经返回 conflict 且无数据，保留该测试断言失败记录。按原能力契约分别断言准确错误码后，最终回归 2 / 0，每项覆盖两侧，并确认恢复后的新请求可以成功。

本机受影响回归：filesystem_deadline_tests 8 / 0；request_deadline_tests 22 / 0；最终 MCP Clippy 通过。此夹具不证明原生扫描。Windows 正在运行的完整任务使用此前 CLI 修复源码，不包含本次 MCP 修改，不将其结果记为 MCP 新源码验收。

Windows 已结构化更新上述六个 MCP 文件，实际部署绑定不变。默认并行 MCP 全 lib 223 项：219 通过 / 4 失败 / 0 忽略，45.62 秒（含构建整次 142.359 秒）。新增 growth 撤权恢复回归通过；新增 history 已拒绝原请求，但恢复后的新请求预算耗尽，整体仍失败。其余三项分别是捕获权限撤销期待 PermissionDenied 实得 BudgetExceeded、过期期限合法部分诊断 BudgetExceeded、损坏数据期待 internal_error 实得 budget_exceeded。没有通过修改断言、期限或串行化消除这些失败。

同批 CLI 定向选择器误用 --lib（该 package 仅有 binary），未执行 CLI 测试；该失败单独保留，后续使用真实 --bin diskgraph 纠正。Windows 原始日志、receipt 与 source.diff 位于台式机临时目录 cli_mcp_encoding_native_final，变更摘要 dafeb2fd0b39c2f408a2965336fe5c6794b6c4f296ad767273ad9852a3b74edd。后续 MCP 定向启用诊断，只用于定位预算阶段，不替代此次默认全量失败。

后续 cli_mcp_encoding_native_diagnostic：纠正选择器的 CLI 默认环境/默认并行 7 / 0（4.39 秒）；MCP 开启诊断的撤权恢复 2 / 0（1.60 秒）、filesystem_deadline_tests 8 / 0（1.42 秒）。本次定向未产生预算阶段诊断，不据此推断此前失败原因或撤销默认全量反例。17 个修改源文件摘要与本机一致；记录在 docs/benchmarks/encoding_withdrawal_2026_10_09/receipt.json。

状态（上一轮）：历史两项生产路径已有定向安全回归；关系类尚待本轮修复。后续证据见下，不撤销此前全量失败。

CLI 关系接口补充验收：related/explain/impact/candidates 必须在实际 envelope 编码时撤权并恢复后拒绝原请求；使用原生扫描的 Cargo 项目关系，不以空实体或预查询错误代替编码阶段。旧请求不输出任何文本，原始期限和现有 wire 字段保持。

## 关系响应实际编码修复与 Windows 原生验证

MCP 四类接口先以真实 bearer/Origin/TCP 请求复现 0 / 4，撤权恢复后均实际返回成功数据。Engine 增加保持旧签名兼容的 with_finish 回调接口；MCP 与 CLI 在原请求撤权见证内完成实际 envelope 编码，终检通过后才发布文本及结构化数据，不重新捕获见证。typed candidates/impact 的 JSON 构造也位于同一回调内。CLI candidates 范围检查和服务器标识读取沿用原期限。期限到达仍保留原 truncated/complete 字段和响应字节限制。

Windows 使用真实 Cargo 项目扫描的四项 CLI 回归先 0 / 4，每项原返回 Ok(())，证明编码时撤权恢复未被原请求记住；修复后同批 11 项编码撤权回归 11 / 0，完整 CLI binary 88 / 0（14.18 秒）。绑定实际扫描 worker、driver 及已编译原生退出状态夹具，使用默认并行，未修改默认请求期限。

Windows MCP 新增四项撤权恢复、四项编码超期诊断回归 8 / 0（3.12 秒）；完整 MCP lib 231 / 0（21.53 秒），默认并行。没有将上一轮 219 / 4 删除或改成通过；当前单次成功尚不足以证明重复稳定性。macOS 查询安全夹具 8 / 0，filesystem_deadline_tests 8 / 0，CLI/MCP/Engine Clippy 通过。

本机与 Windows 14 个修改源文件摘要完全一致；执行任务、原日志摘要和哈希记录在 docs/benchmarks/relation_encoding_withdrawal_2026_10_09/receipt.json。Windows 完整日志保留原生临时目录，未用摘要冒充原文件导出。当前关系类安全实现与上述定向/模块回归已完成，完整 workspace、原阈值性能、所有平台 CI 和恢复监督验收仍开放。

当前 c3ff925d CI 的三项 Linux workspace 失败均含 CLI 旧七项回归强制期待 PermissionDenied，实际返回 Conflict 且无数据；与 MCP/Engine 既定能力契约不一致。CLI 测试改为实际探测 watch_authorization_withdrawal：撤权恢复且无原生精确见证时必须准确返回 Conflict；有原生见证或未恢复授权时仍必须 PermissionDenied。不接受 BudgetExceeded/Timeout/任意错误作为撤权通过，不更改生产授权逻辑。原 CI 红灯日志保留。

按原生能力契约修正精确错误码断言后，Windows 当前源码再次执行 11 / 0（4.85 秒）、完整 CLI 88 / 0（14.96 秒）；macOS 原 Engine 连续撤权回归 6 / 0、请求期限回归 22 / 0、CLI Clippy 通过。最终摘要见 windows_capability_contract.json；之前 CLI 原断言更强但平台不兼容的 88 / 0 保留为上一执行版本，当前 14 文件源码一致性使用最新回执。Linux 断言修正后的原生结果仍等待新的 CI，不把 Windows 结果替代 Linux 验收。
