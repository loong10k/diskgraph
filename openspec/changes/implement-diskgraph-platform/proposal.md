## Why

智能体目前需要反复遍历目录、拼接空间统计和人工判断文件归属，既浪费调查成本，也缺少跨会话、跨机器可复用的证据。DiskGraph 已有四个 Rust crate 的只读基础，需要演进为可独立安装的共享引擎，同时支撑 PruneX、通用智能体和服务器上的受控文件管理。

## What Changes

- 明确产品关系：PruneX 是图形产品层，DiskGraph 是共享 Rust 引擎；CLI、FFI、MCP 是并列入口，核心不依赖模型或 PruneX。
- 继续以固定修订依赖 `disktree-core`，不复制上游源码、不要求安装 disktree 应用；基础文件能力使用 Rust/平台 API，专业生态通过可选适配器接入。
- 在保留 v1 行为的基础上设计显式 v2：无损资源定位、扫描覆盖、版本化实体/证据、SQLite 快照与历史变化、有界查询和索引容量治理。
- 提供 29 个业务命令/命令族及 CLI/MCP/Rust/FFI 能力映射；完整能力不因 MCP 的默认工具展示策略而减少。
- 新增 Rust `diskgraph-mcp`：stdio、Streamable HTTP、旧版 HTTP+SSE 兼容入口共用服务；旧版入口独立适配、默认关闭但纳入验收，不以新 HTTP 内部 SSE 代替。
- 新增可选 `diskgraph-ops`：移动、复制、回收、恢复、永久删除统一经过计划、可信批准、执行前重验、幂等执行与结果记录。修正早期“所有执行仅属于 PruneX”的架构假设；查询层仍不能执行这些动作。
- 支持服务器本地索引与执行、本地智能体远程查询；默认只读，身份、目录范围、内容读取、索引管理和写操作分别授权。
- 规划元数据归属、按需内容读取/去重、Git/进程证据、Cargo/Docker 等生态能力；不引入任意 Shell 或自动提权接口。
- 规划 macOS/Linux/Windows 本机交付、PruneX Swift/Kotlin 集成、Android URI 与 iOS 受限文档；能力按平台声明，不承诺移动端全盘管理。
- 建立 TDD、协议/权限负向测试、故障恢复、真实客户端及真实平台验证、智能体效率评测与私有发行门禁。

本次只产出文档和规划；不实施上述代码、数据库迁移、文件操作、客户端安装或远端部署。仓库保持私有，不创建分支或发布。`v1` 不被无声替换；新 schema/API 使用显式版本及迁移，旧程序不得误读升级库。

## Capabilities

### New Capabilities

- `scope-authorization`: 本机/服务器身份、显式目录范围、最小权限、内容和写操作授权。
- `filesystem-snapshots`: 原生及 URI 观察、无损身份、尺寸口径、覆盖、增量失效。
- `relationship-evidence`: 项目/应用/进程/重建/保护关系、证据来源、冲突与时效。
- `snapshot-storage`: SQLite 版本、迁移、索引、不可变快照和保留依赖。
- `bounded-queries`: 查询契约、聚合 explore、分页、历史比较、候选及未知状态。
- `content-inspection`: 有界文件内容读取、按需重复内容核验、云端占位符保护。
- `command-surface`: 29 个业务入口、参数与输出协议、跨入口一致性。
- `mcp-transports`: 三种传输、版本兼容、服务端部署、工具能力配置。
- `safe-file-operations`: 计划、批准、重验、移动/复制/回收/恢复/永久删除。
- `ecosystem-adapters`: Git/进程证据及可选 Cargo/Docker 等专业工具策略。
- `agent-integration`: 独立安装、智能体配置、共享索引、使用指导和实际宿主验证。
- `platform-ffi`: Rust/Swift/Kotlin 共用核心、平台能力、Rust 图库/控制库与 PruneX 业务库共存。
- `runtime-governance`: 作业、租约、取消、幂等、审计、恢复记录与容量限制。
- `release-evaluation`: 测试、威胁模型、性能/效率对照、私有发行与平台验收。

### Modified Capabilities

无既有 `openspec/specs`。以上是对现有源码基础和新增行为建立首套正式规格，不表示当前实现已经满足规格。

## Impact

- 现有模块：`diskgraph-core`、`diskgraph-store`、`diskgraph-disktree`、`diskgraph-ffi`；预期新增 engine、collectors、ops、cli、mcp 模块/crate。
- 依赖：保留已固定的扫描库；实施时核对 Rust MCP SDK、HTTP 服务、平台权限/回收接口、SQLite 与原生宿主链接方式，不在本次安装依赖。
- 数据：图索引 `diskgraph.sqlite` 与不可随重建丢失的控制/执行记录分离；PruneX 会话与 UI 数据仍归 `prunex.sqlite`，服务端记录才是远程执行权威。
- 外部系统：真实文件系统、可选 Git/Cargo/Docker、智能体配置、HTTP 身份服务和原生应用。全部修改能力默认关闭，按阶段通过门禁才启用。
- 事实源：本 change 的 delta specs 定义需求与验收，design/tasks 定义实施；`docs/architecture.md`、`docs/technical-design.md` 和 `docs/command-reference.md` 是解释性文档，发生分歧先修订本 change，不能创建另一套冲突规格。
