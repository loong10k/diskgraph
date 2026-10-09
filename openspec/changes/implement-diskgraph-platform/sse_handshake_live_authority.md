# SSE 成功握手前的实时授权

沿用 mcp-transports 与 scope-authorization。现代与启用的 legacy SSE 在发送 HTTP 200、分配主体连接额度或签发会话端点前，必须沿原 50 ms 有界观察确认 token 候选权限仍有持久实时授权。已认证但没有实时授权返回 403；控制锁、SQL 或存储错误导致无法确认时返回 503，不签发可见会话。Origin、认证、token 期限、禁用 legacy 的响应和可信本地模式保持原契约；连接存活期间仍逐轮复验，不能缓存握手允许结果。

验收采用真实 socket，覆盖现代与 legacy 两条路径：有效 token 无 grant 时，响应不得包含 200、事件流类型或 endpoint；原控制 owner 持锁直到响应收齐时，必须返回 503 且没有会话。原实现先取得行为 RED，再接入统一存活观察结果；既有 SSE 撤权、原 owner 竞争、四槽回收、噪音输入与 HTTP/MCP 全目标回归继续有效。此项不关闭默认并发稳定性、原 200k 门禁或恢复监督父项。

实现：共享服务提供 Result<bool> 的有界身份观察，既有布尔周期检查仍将错误视为不可继续。统一 GET 分发在认证/Origin 之后、主体槽分配之前检查；未启用 legacy 的 404 和可信本地路径不改变。不能将握手观察解读为后续授权不可撤销，原周期 SQL 检查仍执行。

Windows 原生 TDD：旧实现两项目标回归 0/2，均真实收到 200 事件流头；已应用实现后 2/0（现代和 legacy 各覆盖无授权、控制 owner 原持锁，共四种 socket 情形）。第一次远端事务编辑因两处相同定位文本而全部拒绝，期间一次命名为 green 的运行实际上仍运行旧逻辑，不计作修复后结果；修复定位并确认改动后才取得本段 GREEN。源码、worker/driver、原 argv、前后源码一致性与日志摘要保留在 windows_sse_handshake_{red,green}_2026_10_09.json，GREEN 四个修改源码摘要已与本机逐项核对相等。

Windows 默认并发 CLI/MCP 全目标 29 个目标均成功：CLI 单元 88/0、MCP 单元 233/0，其余 CLI/MCP 集成全部通过，两个零测试目标不计作测试覆盖。完整回执 windows_sse_handshake_frontends_2026_10_09.json 保留每个目标计数及原始日志位置/摘要。未更改原授权或查询期限、测试线程数、失败断言和 200k 门禁。MCP 全目标严格 Clippy、定向 rustfmt、diff 检查通过；首次离线 Clippy 缺 Cargo.lock 固定 hmac 依赖，补取固定依赖后完成，不计作行为 RED。

验收边界：这是一轮实际 Windows 默认并发通过，不足以撤销此前间歇性失败或关闭稳定性父项。macOS 本机缺受信宿主部署，Linux/macOS 当前 CI 尚未包含本改动；CI 37872537642 的 Windows 包 200k 300 秒门禁仍失败。恢复监督及总体平台任务未勾选。
