## Purpose

使同一个 DiskGraph 服务通过本地 stdio、远程 Streamable HTTP 和旧版 HTTP+SSE 兼容模式供智能体调用，同时维持统一业务、权限和可观测执行状态，而不把流式响应或网络重连误当成执行完成证明。

## ADDED Requirements

### Requirement: MCP-01 Three transport forms
系统 SHALL 支持 stdio、Streamable HTTP 与独立的 legacy HTTP+SSE 兼容入口；旧版兼容默认关闭但有真实旧客户端契约测试，不能用现代 HTTP 中的 SSE 响应冒充兼容。

#### Scenario: Legacy only client
- **WHEN** 管理员显式启用兼容入口后旧客户端连接
- **THEN** 按该版本双端点等实际契约完成工具调用，与现代入口获得相同授权后的结果。

#### Scenario: Legacy disabled
- **WHEN** 客户端访问未开启旧版入口
- **THEN** 明确拒绝，不静默启动兼容服务器。

### Requirement: MCP-02 Protocol clean stdio
stdio SHALL 将 stdout 专用于协议消息，日志与进度不得污染；协议版本按受支持客户端协商或检测，不能硬编码所有版本共享同一生命周期。

#### Scenario: Verbose logging
- **WHEN** 启用调试日志后执行 stdio 查询
- **THEN** 协议流仍可解析，日志不进入 stdout。

### Requirement: MCP-03 Remote service locality
HTTP 模式 SHALL 在服务器本机管理索引和文件操作；返回 server/scope/operation 等标识，客户端无需安装扫描库或挂载服务端数据库。

#### Scenario: Local agent remote files
- **WHEN** 本地智能体查询另一台机器的目录
- **THEN** 执行发生在该服务端授权范围内，本地同名路径不受影响。

### Requirement: MCP-04 Uniform enforcement and tool groups
所有入口 SHALL 共用 read、manage、write 等能力配置和逐请求权限；协议注解、工具隐藏和客户端确认均不能代替服务端强制策略。

#### Scenario: Hidden tool direct call
- **WHEN** 只读客户端直接发送写工具调用
- **THEN** 按权限拒绝；更换为旧版 SSE 不能绕过。

### Requirement: MCP-05 Reconnect and task identity
长任务 SHALL 返回持久 job/operation ID，并在连接重建后允许授权主体查询；传输超时、断线、协议取消和业务终态必须区分。

#### Scenario: Disconnect after apply
- **WHEN** 服务端可能已执行而客户端未收到结果
- **THEN** 重连查询原操作，不能自动创建第二次删除或宣称已回滚。

#### Scenario: Reconnect after the job has completed
- **WHEN** 客户端获得持久 job ID 后断线，而任务在新的状态查询到达前已经完成
- **THEN** 新连接可以首轮直接观察 completed，不要求客户端先看见 queued 或 running；再次独立连接查询仍返回同一个 job ID 的 completed 状态，不能因原连接关闭丢失结果或创建替代任务。

### Requirement: MCP-06 Network controls
远程服务 SHALL 要求认证、配置的 TLS/可信加密隧道、Origin 策略、请求体/响应/连接预算及限流；服务监听和反向代理信任边界必须显式配置。

生产服务配置 SHALL 从受限文件读取签名密钥，不得将密钥展开到服务进程参数；旧内联参数保留兼容但不得用于交付示例。`diskgraph serve` 在 Windows SHALL 正确定位 MCP `.exe` 子进程。

#### Scenario: Hostile origin or forged identity
- **WHEN** 请求带不允许的 Origin 或未验证身份头
- **THEN** 拒绝请求且不执行工具。

#### Scenario: Oversized or slowly streamed headers
- **WHEN** 未认证连接持续发送超大请求行或请求头，或仅滴流字节以保持连接活动
- **THEN** 在有界总头字节、单字段长度与绝对期限内关闭连接，不占满服务工作线程。

#### Scenario: Connection cap response and bounded shutdown
- **WHEN** 连接上限已满且客户端发送普通完整请求
- **THEN** 返回完整 503 与 Connection: close，再半关闭写端并以 50 ms 绝对期限、64 KiB 接收清理预算释放连接；不启动工具、业务工作线程或无限等待客户端。超过清理预算的输入允许直接终止，不承诺完整错误响应，OS 调度不属于严格墙钟上限。

#### Scenario: Bounded client rate state and monotonic refill
- **WHEN** 未认证请求不断更换客户端地址，或时钟观测倒退、请求并发导致较早观测延后进入限流锁
- **THEN** 限流表最多保留 4096 个客户端、单个键最多 64 字节，满表拒绝新客户端而不逐出仍活跃的额度；60 秒未访问且已经完全可补充的桶允许回收，清理最多每秒一次，补充基于单调时钟且旧观测不重复产生额度。满表和非法键均返回现有 429/retry_after_ms 语义，不进入认证或工具执行。

#### Scenario: Canonical client IP and trusted proxy boundary
- **WHEN** 客户端使用 IPv6、IPv4 映射地址或通过可信代理发送 X-Forwarded-For
- **THEN** 限流与诊断使用规范化 IP，不混淆端口或 IPv6 分段；重复 X-Forwarded-For 字段按接收顺序合并且受总头字节预算约束；仅接受由实际可信 peer 传递的最多 32 个合法 IP，从右向左跨越可信代理，停在第一个不可信 hop，不能用客户端伪造前缀刷新额度。无效或过长链回退到实际 peer，转发头不建立授权主体。

#### Scenario: Legacy response and queued delivery budgets
- **WHEN** legacy SSE 产生超过配置响应上限的结果，或慢消费者使待发送数据积压
- **THEN** 与现代 HTTP 执行同一响应字节门禁，累计队列按实际字节和条数有界；拥塞在执行工具前明确拒绝，不能发送 202 后静默丢弃结果或无限分配。超限结果使用有界协议错误，断线、撤权和发送失败释放所有预留资源；可信内部兼容接口不能被远程路径用于绕过预算。
- **AND** 待执行/编码/排队/发送中的 legacy 结果共享每会话 64 条、16 MiB 和每监听实例 64 MiB 的投递额度（包含 SSE 包装），执行前按配置响应上限预留，编码完成后按实际字节缩减。容不下关联请求 ID 的最小错误或单条预留超过会话上限时先返回 413；拥塞先返回 429。部分写入以绝对期限重试并重新检查身份与控制库 v6 的授权变更计数（policy/grant/scope 的事务触发器，独立于策略 epoch）；授权改变时保守关闭旧结果流，任务心跳不触发该计数。此门禁限制投递数据与预留，不等价于工具内部 Value 或严格 RSS 上限。

#### Scenario: Remote serve from the CLI
- **WHEN** 本机管理员通过 `diskgraph serve` 指定远程认证与 Origin 配置
- **THEN** 配置完整传给 MCP 子进程，服务正常启动且无 token 请求被拒绝。

#### Scenario: Protected verifier key and Windows delegation
- **WHEN** 部署使用受限密钥文件，或 Windows CLI 启动本机 MCP 子进程
- **THEN** 验收进程真实完成授权调用，密钥不出现在子进程参数中；不安全权限、空密钥或缺失文件使服务启动失败。

#### Scenario: Strict serialized Origin and canonical loopback addresses
- **WHEN** a request supplies an Origin with a path, query, fragment, userinfo, malformed IP/brackets, or invalid authority
- **THEN** every HTTP/SSE entry refuses it before authentication or tool execution, even when the same malformed string appears in the configured allowlist
- **AND** ordinary canonical IPv4 and bracketed IPv6 loopback origins with or without a port remain accepted; remote origin allowlist comparisons remain exact
- **AND** malformed or abbreviated numeric bind hosts do not receive the trusted loopback exemption
- **AND** the raw HTTP reader preserves non-ASCII field values for rejection, trims only ASCII OWS, and rejects malformed field-name tokens and forbidden field-value controls before dispatch

#### Scenario: Invalid UTF-8 request bodies are refused before dispatch
- **WHEN** an HTTP request body contains malformed UTF-8, including malformed bytes inside an otherwise valid JSON string
- **THEN** the shared HTTP reader refuses the request as invalid data before modern or legacy protocol dispatch, without lossy replacement or logging the body
- **AND** valid UTF-8, including Chinese, emoji and an explicitly encoded U+FFFD character, is preserved byte-for-byte in the decoded request; original byte and absolute-time budgets remain unchanged

#### Scenario: Content-Length uses decimal framing without numeric extensions
- **WHEN** a request supplies an empty, signed, non-decimal or overflowing Content-Length
- **THEN** the shared HTTP reader refuses it before body allocation and dispatch; integer parser extensions such as a leading plus sign cannot establish framing
- **AND** zero and digit-only leading-zero values retain their existing semantics, subject to the original body byte and absolute-time limits
