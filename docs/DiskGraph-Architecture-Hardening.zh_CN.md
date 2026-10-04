# DiskGraph 已实现的安全与查询边界

本文延续[总体架构](DiskGraph-Architecture.zh_CN.md)，设计目标仍须对应平台验收。

## 2026-10-01 加固实现边界

以下为本轮已落地调用链；前述标记 Target 的总体路线仍需对应平台验收。

```mermaid
sequenceDiagram
    participant R as HTTP / SSE 请求
    participant A as Origin + Token 校验
    participant E as Engine 请求授权
    participant C as 控制库
    participant G as 独立图库读连接
    R->>A: 请求与认证信息
    A->>E: 不可变主体、能力、传输、到期时间
    E->>G: 查询 revision 实际 server / scope
    E->>C: 读取实时策略与撤权状态
    C-->>E: 数据库授权
    E->>E: token 能力 ∩ 实时授权
    E->>G: 有期限的窄读 / 有界路径合并
    G-->>R: 数据与截断诊断
```

写入由串行图库连接执行。job 以条件 UPDATE 认领，控制库事务内校验租约、fencing、scope 和实时 IndexWrite，保护 staging 批次、revision 发布和 collector 写入。过期认领使用新 staging 命名空间并重扫。上游扫描器源和摘要保持原 pin；转换使用迭代遍历，发布从 staging 生成正式节点。

迁移前使用 SQLite 一致性备份（包含已提交 WAL）至 `migration_backups/`；图库 schema 6 记录归属及规范化搜索字段，schema 7 增加未知大小稀疏索引、关系分页与有序路径索引，schema 8 增加候选目录大小与证据关系索引。Schema 9 在每次发布事务内维护精确快照/目录计数及尺寸累计计数，升级事务回填；snapshot writer 标记拒绝仍打开的旧程序写入，计数元数据缺失时拒绝查询。Known/unknown 页保留旧 JSON fallback 语义并走对应 partial index，显式 OFFSET 仍需 O(offset+page)。聚合增加存储和发布/迁移成本，备份/WAL/临时文件测量见[全平台记录](production-readiness-full-platform-2026-10-02.zh-CN.md)。候选选择和影响遍历使用请求专用读连接、期限与明确截断诊断。控制库 schema 4 记录租约/fencing，schema 5 持久化取消意图。控制库 schema 6 使用事务触发器记录独立授权变更计数，覆盖 policy/grant/scope，单条 grant 撤销也会变更；任务心跳不变更该计数。v5 升级前进行一致性 pre-v6 备份，失败原子回滚；升级前先停止旧服务，已经打开的旧连接不会自动取得新传输行为。旧 running job 等待 heartbeat + 30 秒租约到期，不在启动时抢占。图库 WAL/NORMAL 保证事务一致性，但断电可能丢失最近提交的可重建索引；控制库 FULL 保持操作记录持久性要求。两库仍无跨库原子事务承诺。

```mermaid
flowchart LR
    X["取消或撤权"] --> C[("控制库：持久意图")]
    C --> F{"租约 + fence + 实时授权"}
    F -- 有效 --> S["写当前 staging 命名空间"]
    F -- 取消 / 失效 --> R["停止；旧 revision 不变"]
    S --> F2{"发布 fence"}
    F2 -- 有效 --> P["原子发布 revision"]
    F2 -- 取消 / 失效 --> R
    O["已批准文件计划"] --> V["实时授权 + 源指纹"]
    V --> A["Immediate 事务：路径 + 文件身份认领"]
    A --> H["句柄约束、禁止覆盖操作"]
```

FFI 从无损图库路径派生控制数据隔离域；归属不明的旧共享控制数据拒绝复用。操作计划要求完整摘要和新鲜源证据，因此旧计划须重新创建。正目标候选现已采用有界准备，关系 impact 在分页和方向之间共享同一授权读连接。显式 offset 兼容输入仍需要遍历偏移；扫描取消不提供严格 RSS 上限。

兼容入口、保真检查、扫描超限余量、历史物理容量及本机测量详见[中文验收记录](security-performance-hardening-2026-10-01.zh-CN.md)。CLI/MCP 写工具关闭；Linux/Windows 原生操作与严格扫描 RSS 上限不在已完成能力中。


## Legacy 结果投递边界

```mermaid
flowchart TD
    P["POST /messages<br/>Origin + token + 会话主体"] --> A{"关联错误可容纳？<br/>字节 + 条数预留"}
    A -->|拒绝| R["413 / 429<br/>业务执行前返回"]
    A -->|准入| H["202 + 授权变更计数"]
    H --> E["McpService<br/>token 能力 ∩ 实时 grant"]
    E --> J["有界 JSON 编码<br/>结果超限 → 协议错误"]
    J --> Q["按实际帧字节缩减<br/>有界会话队列"]
    Q --> W["分段 socket 写入<br/>到期 + 授权计数 + 绝对期限"]
    W --> D["销毁帧并归还额度"]
    Q --> C["断线 / 撤权 / 发送失败"]
    C --> D
```

远程 legacy 使用私有预算注册表，公开裸 sender 仅保留为可信内部兼容接口。每会话 64 条/16 MiB、每监听实例合计 64 MiB，包含待执行、编码及发送中的预留；准入先按 `max_response_bytes + 22` 认领，编码后缩为实际帧。默认 4 MiB 允许每会话三个、全实例十五个同时执行的最坏预留。无法容纳关联错误或单条预留时先返回 413，拥塞先返回 429；共用有界编码器保留现代 HTTP 的字段与超限状态。它约束投递缓冲，不约束工具 Value、分配器、内核缓冲或严格 RSS。

数据帧每次分段写检查到期与持久授权计数；非阻塞控制锁准入、SQLite 等待/执行和 socket 写入共用同一绝对期限。任何授权相关变更（包括无关主体、新增 grant）都会保守关闭旧结果，任务/操作表不触发；已经交给内核的字节无法撤回。外层闲置存活检查仍可能等待共享策略访问，没有严格一秒撤权/清理 SLA。接收端关闭释放排队帧，仍在运行的业务继续占用全局预留直到退出。202 后若断线则明确关闭并记录，持久 job 可重查。升级必须先停止旧宿主，不能用 schema 重开门禁冒充对存活旧进程的修复。

### Windows 普通文件内容获取

```mermaid
flowchart TD
    A["Engine content:read<br/>真实主体 + 实时授权"] --> H["HydrationGuard<br/>当前线程暴露占位属性"]
    H --> P["WindowsPathPlan<br/>本地 drive + 精确 scope 组件"]
    P --> D["WindowsScopedFile<br/>保留父句柄；每次相对打开一个组件"]
    D --> S["WindowsFileState<br/>仅属性句柄 + 完整原生身份"]
    S --> G{"普通文件<br/>无 reparse / offline / recall?"}
    G -->|拒绝| R["Placeholder / unsupported / conflict<br/>无内容或确认摘要"]
    G -->|准入| F["同一持有父目录下的数据句柄<br/>只共享读取"]
    F --> B["ScopedContent + 有界读取/摘要<br/>预算、撤权、取消、版本核验"]
    B --> X["释放数据及目录租约<br/>恢复线程模式"]
```

此增量保留公开结果字段及 Unix 路径。路径规划直接拒绝 ADS、父级跳转、UNC/设备命名空间及过长输入，不对客户端路径执行 canonicalize。完整 128 位 file ID 与原生写入/变更版本保持私有，不截断填入现有快照 ID 字段。属性获取不冻结新 writer：变化在数据访问前以 Conflict 拒绝；数据句柄随后拒绝普通写入/删除共享，持有父目录防止替换。这不构成原子快照或对所有 mapping/kernel/filter 活动的冻结；100ns 是表示单位，不保证文件系统实际精度或单调版本。可选线程 API（Windows 10 1709+）动态解析，缺能力返回公开 unsupported 错误码。线程模式不覆盖 scanner worker，打开时 no-recall 标志也不能证明真实 provider 后续读取不下载。取消/期限是协作式，不能抢占同步原生 I/O。原生回归证据限 CI 的 NTFS 夹具，其他文件系统/provider 验收分别记录于[全平台记录](production-readiness-full-platform-2026-10-02.zh-CN.md)；公开文件写能力继续关闭。


## Git 实时证据输入隔离（EC-04 / D20）

可信库采样使用私有配置、index、引用及扁平对象视图。D20时点调用来自库/测试；下文D39记录CLI/MCP产品接线，FFI采样集成仍是独立事项。普通loose对象和配对pack/index经no-follow捕获及同一私有分配owner复制；源alternates/promisor和不支持输入明确拒绝，忽略的加速文件不交Git。来源对象和元数据在准备与终检共享累计64 MiB/32k额度，私有owner另核128 MiB对象报告分配与64 MiB卷余量；ProbeLimits仍默认整次协作式15秒和累计输出1 MiB。

```mermaid
flowchart TD
    Q["可信库请求"] --> C["原生 no-follow 捕获<br/>配置 / index / 引用 / 普通对象"]
    C --> P["私有 owner<br/>复制文件，不硬链接或挂源 alternate"]
    P --> G["固定受信 Git + 私有 GIT_DIR<br/>共享期限 / 输出 / 取消"]
    G --> V["核验来源字节和版本<br/>核验私有文件与实际分配"]
    V --> X["显式清理"]
    X --> R["完整样本或明确错误"]
    C -->|不支持或预算耗尽| X
    P -->|容量失败| X
    G -->|失败或取消| X
```

私有视图关闭递归源对象输入，但不是原子仓库快照或完整文件系统/RSS沙箱。复制和复核增加随字节量变化的成本，初始buffer保留及终检读取会增加内存；大pack或status输出可能明确超过默认额度。本机回归、复制成本原始测量与原生验收分别记于[全平台记录](production-readiness-full-platform-2026-10-02.zh-CN.md)。公开危险写工具仍关闭，其余设备/provider/生产门禁保持原要求。

## 树与历史请求边界 — D24

Store 入口只声明模块与导出。树窗口和历史有序游标先借用 SQLite 原始字段、
检查预算后再分配/解码；历史双侧及同步计划必要的窄重读共用一个账本。编码后
复核双方实际 revision 归属，最后能力回调结束后再检查所有持久授权。数值
minimum 计数保留既有累计索引和未知大小诊断。晚到报告明确局部统计，晚到计划
直接拒绝；错误诊断计入实际 JSON 转义且保留原业务错误码。正文读取在 EOF、
精确范围及身份末检之后检查 scope、授权和取消；旧读取包装新增 30 秒协作默认，
`read_bounded_until` 继承调用方期限。这些检查不承诺跨连接授权原子性、严格 I/O
时间或 RSS 上限。[全平台验收边界](production-readiness-full-platform-2026-10-02.zh-CN.md)
仍分别记录。

```mermaid
flowchart LR
    R["请求身份 + 单一期限"] --> A["实际 revision 归属"]
    A --> Q["有界 SQL + 共享原始账本"]
    Q --> J["有限 JSON 编码"]
    J --> F["全部能力检查后<br/>复核双方持久授权"]
    F --> O["响应或拒绝<br/>计划要求完整结果"]
```

## 历史尺寸资格 — D33

Core 统一维护增长、变化与比较使用的纯函数。仅 `size_known && !read_error` 表示尺寸已观察；数值增量还要求准确节点类型相同。Engine 比较行按每侧独立保留未知值，只统计实测 `Size` 或 `Contents` 尺寸变化；跨根元数据比较保留原有相对路径契约。SQL 准入、共享预算及授权末检位于资格判断之外，`None` 不能替代取消、到期或权限错误。查询/比较对象真实分文件，保留既有公开导出与序列化类型。

```mermaid
flowchart LR
    A["实际归属 + 共享预算"] --> R["有界历史读取"]
    R --> Q["已观察尺寸 + 准确类型"]
    Q --> V["有符号增量或不可用值"]
    V --> T["编码 + 授权末检"]
```

尺寸变化数为零不能证明未知对象未变。同 scope 的 Q-04 兼容矩阵与一致完整覆盖语义已在 D35 任务3.9、`2450ab1` 同源码原生 CI 中验收；Windows 历史身份连续性及历史正文版本绑定仍是独立未完成项。

## 历史命名空间资格 — D34

Engine 增长与变化复用已有 reader 核对 revision 持久归属。两侧须属于本服务器及同一有效 scope；注册 API 保证每个 scope 的无损根不可变。显示文本不能证明命名空间相同。不同 scope 返回不可用增长，或保留 `different_root` 变化标签并新增 `scope_changed: true`；未绑定、异服务器、撤销及存储失败仍是错误。可信资格辅助方法不授予权限，各入口保留原授权与响应预算，包括编码后的双侧复检。旧 FFI growth 使用同一资格判断，保留导出契约；通用元数据比较在双侧授权后仍允许跨根。

```mermaid
flowchart LR
    A["授权两侧实际 revision"] --> N{"持久 server 与 scope 相同？"}
    N -->|是| H["历史事实与有界读取"]
    N -->|否| U["不可用增长 / scope 诊断"]
    H --> F["预算内编码"]
    U --> F
    F --> T["复检双侧授权及原期限"]
```

命名空间资格不能证明历史文件身份连续或正文版本相同；原生 CI 与这些剩余要求独立记录。

## 受约束 Git 捕获与 TUI 准入 — D36

可信库入口 `sample_git_scoped` 接受已注册根和明确的无损仓库定位。保留根句柄、原生相对组件打开在启动子进程前约束工作树及 Git 依赖；独立私有捕获提供普通文件、元数据、属性和对象依赖，支持的命令不再打开实时工作树。捕获和复验共用输入、条目、取消及时间预算，终检拒绝源变化和根路径链替换。嵌套仓库、scope 外依赖明确不支持。调用者仍须授权正文访问：独立 D39 产品入口接入持久 collector job 与 CLI/MCP 命令，见下文。

```mermaid
flowchart LR
    R["已注册根 + 明确定位"] --> H["保留根句柄；原生相对打开"]
    H --> C["共享预算内捕获普通输入"]
    C --> P["独占私有工作树及 Git 元数据"]
    P --> G["固定支持的 Git 命令"]
    G --> V["源、根路径链及预算复验"]
    V --> X["显式清理；返回样本或错误"]
```

TUI 导航与整帧准备采用专用 display reader。初始控制锁争用及时拒绝，实际 revision 归属和实时授权在原读取期限内检查。导航只投影所需字段，借用原始数据先准入再解码，续页探针不读取 payload。整帧各层共用读取账本，保留的展示数据另计预算；完整页须通过末段授权及原期限检查才返回。明确截断的帧可在末检成功后保留已绘数据，撤权及真实存储故障仍返回错误。

```mermaid
flowchart LR
    A["尝试控制锁；授权实际 revision"] --> B["原期限内读取"]
    B --> Q["借用数据准入；所需字段投影"]
    Q --> F["准备页面或内存画布"]
    F --> T["尝试控制锁；复检实时授权"]
    T --> D{"完整结果符合原期限？"}
    D -->|是| O["交付完整页面或帧"]
    D -->|明确截断的帧| P["交付标注截断的部分帧"]
    D -->|错误或完整结果超时| E["丢弃准备结果"]
```

候选准备也在解码前准入快照覆盖头，与被选节点和必需证据共用原始字节账本。原请求已到期时保留类型化空 `Deadline` 结果，并明确返回 `coverage_observed: false`，不能解释为已经观测到覆盖缺口；成功准入的头保留真实覆盖状态。原始字节不足、坏头或缺索引仍返回错误。CLI、MCP 和 FFI 保留旧 wire 字段并新增该诊断；直接构造 `CandidateSelection` 字面量的 Rust 调用者需要补充新字段。

TUI 入口现只包含模块声明和导出，真实对象与绘制逻辑各自分文件。同步 authorizer 仍采用合作检查，不承诺任意回调硬抢占、SQLite C 分配上限或严格 RSS 上限；显式导航 offset 仍需 O(offset + page) 工作。D36 源码及实际平台验收分别记录在 [Git 捕获回执](benchmarks/scoped_git_capture_acceptance_2026_10_04.json)和 [TUI 预算回执](benchmarks/tui_budget_acceptance_2026_10_04.json)。


## 持久 Git 采集 — D39

CLI C03 与 MCP 共用不可变输入和 Engine 授权。控制库 schema8 保存原始输入和有限诊断；图库 schema13 原子发布 collector revision 与唯一回执。基线按实际 server/scope 判断，不采用另一 owner 的 legacy 根指针；丢失选中来源时拒绝。恢复只核对已有回执，不重新采样。排队状态保存在 SQLite，取消标志只属于本机运行代次并按 Arc 身份释放。CLI 状态 data 使用共同授权投影，保留既有外层 scope/revision 字段；MCP 状态内外身份来自同一投影，拒绝不一致 scope 提示。观测指纹覆盖安全摘要和固定请求，不表示全部来源字节的内容哈希。

```mermaid
flowchart TD
    A["CLI sync / MCP diskgraph_sync<br/>collector=git + revision/node"] --> B["Engine<br/>token 能力 ∩ 实时授权 + 原到期时间"]
    B --> C[("控制库：不可变输入<br/>认领 / 租约 / fencing")]
    C --> D["保留 scope 根句柄 + 索引身份<br/>有界私有捕获 / 固定 Git 命令"]
    D --> E["图库 IMMEDIATE 事务<br/>实际归属基线 + 完整来源 + 唯一回执"]
    E --> F["控制库终态协调<br/>核对已提交回执；不重新采样"]
    F --> G["统一授权状态<br/>实际 scope + 回执 revision"]
```

本机最终验证与同源码原生验收分别记录，全平台任务保持开放。

## MCP 服务源码边界 — D40

55行库入口仅包含标准模块声明与明确根导出。`mcp_config.rs`保存配置和默认值，`mcp_service.rs`保留共享Engine与请求上下文；分发、身份/范围解析、真实工具处理及stdio各自分文件，没有新增授权或调度owner。内部可见性维持原crate协作者访问，不增加公开字段。67个原函数正文和21个原测试的有效token一致，包含状态投影和末段授权。

```mermaid
flowchart TD
    T["HTTP / SSE / stdio"] --> S["McpService<br/>共享Engine Arc + 请求上下文"]
    S --> D["service_dispatch<br/>原期限 / profile / schema检查顺序"]
    D --> A["service_identity / scope_access<br/>实际归属 + token能力 ∩ 实时授权"]
    A --> H["快照 / 文件 / 关系 / 管理适配<br/>原处理器与响应末检"]
    H --> E["同一Engine<br/>授权查询与持久任务"]
```

增量AST门禁检查真实标准模块文件、未挂载源、对象/行数/注释/导入/函数正文规则及直接或条件路径覆盖。既有模块明确列为豁免，auth709行、http2657行、protocol538行不因入口变薄而算合规。[D40验收回执](benchmarks/mcp_service_layout_acceptance_2026_10_04.json)分别记录结构负控、实际执行和原生状态。本次整理保留已启用能力，不启用危险文件工具，也不完成平台/provider/宿主/设备/发布门禁。

`fd9330e44318c15db7a9a3ea0cb34e6da2b0e81d`同源码[CI37187379023第二次attempt](https://github.com/loong10k/diskgraph/actions/runs/37187379023)终态为**22/22 success**。保留21项先前成功，仅Kotlin Intel实际重跑；其首次失败发生在宿主调用前的Maven插件描述解析，不能据此推断网络或缓存根因。四份原始workspace日志逐案确认：两个Windows Rust版本各Git157项（2个Unix-only未执行），Linux ARM与macOS Intel各Git159项，以及持久授权39、既有查询32、迁移MCP21、源码门禁9项。这些分组存在重叠。现有Git8.7与请求授权15.20已验收，清单为139完成／28开放／167总项，完整生产目标仍未完成。

## 查询目标准备 — D41，本机检查通过，原生验收待完成

此前TUI／历史消费者的预算没有覆盖其启动前拥有必要`RevisionRecord.snapshot_id`的成本，合法2MiB snapshot ID经真实公开请求复现了该问题。17源增量将归属与snapshot投影分离：先借用准入实际owner字段，核验真实server/scope授权，再准入必要snapshot ID。双侧历史、快照元数据、节点、有序合并及同步计划窄重读继续使用同一`QueryReadBudget`与原期限；命名空间资格直接使用已授权的scope ID，不另读归属。

```mermaid
flowchart LR
    O["借用owner字段<br/>拥有前准入"] --> A["授权实际server/scope<br/>token能力 ∩ 实时授权"]
    A --> S["准入snapshot ID<br/>同账本与原期限"]
    S --> R["窄读 / 合并 / TUI缓冲绘制<br/>消费原剩余额度"]
    S -->|准备失败| T["实时授权末检<br/>历史双侧 / TUI实际scope"]
    R --> T
    T -->|允许且结果可交付| D["返回结果 / 提交画布"]
    T -->|准备失败或拒权| X["传播错误<br/>不提交画布"]
```

双方历史 owner 均已授权后，目标／消费者失败也先执行双侧末检再返回原错误，编码后的成组实时 grant 复验保留。TUI 把已计费账本移入导航／整帧读取；初始目标准备失败不调用 paint，不将缓存伪装为部分帧。既有独立 50 ms 控制终检窗口适用于 Complete、Truncated 与消费者错误结果；Complete（包括导航）必须遵守原读取期限，只有已经绘出明确截断提示的 Truncated 画布可在原图库期限后交付。控制窗口不能续租图库读取。通用可信 reader 及公开兼容包装保持原合同，没有新增 Engine、owner、线程或 schema。

整请求测试覆盖左右大头、累计准备、普通／足额真实成功、初次拒权及末段撤权；TUI 检查实际后端，初始失败必须不绘制／提交。首轮目标 Store 3、Engine 10、CLI 7 通过，随后完整构建发现生产调用的显式导入误受 test 条件限制，仅将该导入改为无条件。修正后 17 源冻结的本机 workspace 1401/0/18、60 suites，fmt／include fmt、严格 all-target Clippy／build、OpenSpec 及 release 通过，vendor 124/0/2 通过。单独执行的 release stdio 18/18、HTTP/SSE 13/13 及真实调用的 UniFFI ABI 19/19 通过；JDK 21 macOS ARM Kotlin 宿主实际通过会话／分页／轮询／v1／release／重开检查，并执行新增 Maven `--errors` 诊断。Kotlin 单独构建的 FFI 库与协议／ABI 库来自同一审查源码，二进制摘要不同，均记录在 [D41 回执](benchmarks/query_target_preparation_acceptance_2026_10_04.json)。新源码原生验收尚未完成，任务 13.6 继续开放。

对额度不足、使用合法 2 MiB snapshot ID 的请求，历史整调用 Rust requested 累计分配从 2,102,370–2,131,834 降至 4,528–4,541 字节，导航从 2,101,248 降至 2,521 字节，整帧从 2,184,272 降至 2,521 字节；初始整帧拒绝不再调用 paint 或提交数据。普通及足额请求仍实际成功。这些隔离分配观察不证明吞吐提升、峰值存活内存、SQLite C 分配、文件系统 I/O 或 RSS 上限；同步授权及原生 I/O 仍为协作检查。历史身份／正文绑定与 provider／移动端／原生写／签名／部署门禁见[最新就绪记录](production-readiness-full-platform-2026-10-04.zh-CN.md)。
