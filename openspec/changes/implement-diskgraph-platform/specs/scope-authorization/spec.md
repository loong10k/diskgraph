## Purpose

定义本机与远程访问资源的身份和授权边界，使目录元数据、文件内容、索引管理及文件修改各自遵循最小权限，并防止跨用户、跨服务器或路径变化导致的数据越权。

## ADDED Requirements

### Requirement: SC-01 Explicit scope registration
系统 SHALL 仅扫描和操作管理员或本机可信用户显式注册的根目录范围；普通查询和清单引用不得自动注册、扩大范围或授予权限。

#### Scenario: Manifest outside the scope
- **WHEN** 项目清单引用授权范围外的输出目录
- **THEN** 保留未解析引用及原因，不扫描或读取该目录。

#### Scenario: Remote root injection
- **WHEN** 远程查询携带未注册的根路径
- **THEN** 返回范围拒绝且不创建 scope。

### Requirement: SC-02 Server and principal bound resources
系统 SHALL 将资源引用绑定 server、scope、revision 与对象 ID，并对每次请求按主体校验范围；知道或猜到 ID 不构成授权。

#### Scenario: Foreign server identifier
- **WHEN** 客户端提交另一服务器的对象 ID
- **THEN** 拒绝该引用，不将其按本地路径解析。

#### Scenario: Denied aggregate
- **WHEN** 无权主体请求其他用户的 top、搜索或统计
- **THEN** 不返回对方名称、数量、尺寸、证据或内容。

### Requirement: SC-03 Separate capabilities
系统 SHALL 默认仅允许已授予的元数据查询，并分别授权 content:read、index:manage、files:move、files:copy、files:trash、files:restore、files:purge 与 admin:scope；FFI、CLI 与所有传输执行同一策略。

#### Scenario: Read only cannot mutate
- **WHEN** 只读凭据直接调用隐藏的 apply 或 FFI 写入口
- **THEN** 服务拒绝执行，不以工具未列出代替权限检查。

#### Scenario: Metadata is not content consent
- **WHEN** 主体只有目录查询权却调用 read
- **THEN** 返回权限不足，不读取文件正文。

### Requirement: SC-04 Revocation and resource boundaries
系统 SHALL 在执行前重新校验撤权、保护规则、挂载身份、源与目标范围，并阻止符号链接、路径穿越或目录替换逃逸；无法满足安全前提的平台不得降级成不受控操作。

#### Scenario: Root changed
- **WHEN** 计划批准后根挂载或中间目录被替换
- **THEN** 计划失效，拒绝继续修改。

#### Scenario: Destination outside grant
- **WHEN** 移动目标不在主体授权范围
- **THEN** 拒绝计划，不因源目录合法而接受目标。

### Requirement: SC-05 Remote authentication and privacy
远程服务 SHALL 对网络请求实施身份认证、逐请求授权、加密传输和 Origin 校验策略，默认不公开监听；索引留在服务器，返回内容遵守导出策略且默认不上传凭据或完整文档。

#### Scenario: Anonymous network call
- **WHEN** 未认证客户端请求 HTTP 查询或旧版 SSE 工具
- **THEN** 拒绝且不泄露索引数据。

#### Scenario: Model exposure
- **WHEN** 云模型宿主仅被授予去标识摘要导出
- **THEN** 响应移除未授权原始路径、内容和秘密，日志同样脱敏。

### Requirement: SC-06 Request-bound identities and revisions
服务 SHALL 对每次远程请求执行 token 能力与实时主体授权的交集，并验证 revision 的持久 server/scope 归属。

#### Scenario: Token without capabilities
- **WHEN** 有效 token 无权限 scope 请求目录或工具
- **THEN** 不返回索引信息，不使用本地管理员身份。

#### Scenario: Foreign revision
- **WHEN** 仅获 scope A 权限的主体携带 scope B revision
- **THEN** 返回拒绝，不泄露 B 的数据。

#### Scenario: Impact traversal with a foreign revision
- **WHEN** 仅获 scope A 权限的主体对 scope B revision 调用关系影响分析
- **THEN** 在读取任何关系前按 B 的实际归属拒绝，不以客户端传入的 A scope 代替 B。

#### Scenario: Revoked queued scan
- **WHEN** 任务排队后 scope 被撤销
- **THEN** 不扫描或发布，并停止已有运行任务。
