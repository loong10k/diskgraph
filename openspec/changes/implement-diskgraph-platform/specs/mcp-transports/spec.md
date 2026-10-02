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

#### Scenario: Remote serve from the CLI
- **WHEN** 本机管理员通过 `diskgraph serve` 指定远程认证与 Origin 配置
- **THEN** 配置完整传给 MCP 子进程，服务正常启动且无 token 请求被拒绝。

#### Scenario: Protected verifier key and Windows delegation
- **WHEN** 部署使用受限密钥文件，或 Windows CLI 启动本机 MCP 子进程
- **THEN** 验收进程真实完成授权调用，密钥不出现在子进程参数中；不安全权限、空密钥或缺失文件使服务启动失败。
