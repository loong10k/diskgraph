## Context

动机与完整范围见 [proposal](proposal.md)。源码基线 `a89e57a` 已包含 core/store/disktree/ffi 四 crate、v1 快照和五种只读查询；没有 CLI、MCP、远程身份或文件执行模块。当前扫描桥接的证据为空、文件身份未填充、路径存在有损转换，Windows 卷身份缺失。

本变更是整个产品路线的分阶段规划，不是一轮即可完成的实现。需求和验收以本目录 `specs/*/spec.md` 为唯一事实源；任务编号见 [tasks](tasks.md)，解释文档为 [架构](../../../docs/architecture.md)、[技术方案](../../../docs/technical-design.md)、[命令参考](../../../docs/command-reference.md)。

## Goals / Non-Goals

**Goals:**

- 独立安装的共享 Rust 引擎，同时服务 PruneX、CLI、本机与远程智能体。
- 版本化目录关系、准确覆盖和时效说明、有界查询与可量化效率收益。
- 丰富业务命令、三种 MCP 传输、一套权限与执行语义。
- 默认只读，可选启用经过批准和重验的操作，具备可审计的失败与恢复边界。
- 从一开始表达移动 URI 和受限能力，分阶段取得真实平台证据。

**Non-Goals:**

- 不构建通用远程 Shell、任意 SQL 控制台、自动提权或全盘无人确认删除器。
- 不让模型成为核心必需依赖，不训练/部署 PruneX Lite/Pro 模型，不建设云端多租户 SaaS。
- 不以图索引代替文件备份；不承诺永久删除恢复、全局文件系统事务或绝对无竞态。
- 不跨机器直接迁移文件，不在移动端承诺系统级清理或其他应用卸载。
- 2026-10-01 按批准计划实现加固代码，仅使用隔离数据库/文件夹具；不迁移真实数据、不清理用户磁盘、不安装宿主配置、不公开发布。

## Decisions

### D1 共享引擎与可选执行分离

core 定义模型/契约/端口，engine 提供查询与索引服务，ops 提供计划与执行；store、scanner、collectors 和平台实现通过端口接入。FFI、CLI、MCP 是并列组装入口，不能相互调用 Shell 或重复实现查询。对照 `PruneX ≈ disktree-app`、`DiskGraph 引擎 ≈ disktree-core`，但不声称功能完全一致。

替代：所有动作留在 PruneX 会使无 GUI 服务器无法独立完成授权后操作；把动作塞进 core/MCP 则扩大只读消费者依赖和风险。因此新增 ops，但按功能配置默认关闭。

### D2 复用 disktree-core，不将系统命令作为基础运行条件

维持固定 Git rev，经 diskgraph-disktree 适配上游类型；修改上游的需求有实际证据后才 fork/vendor，保留 MIT 来源和差异记录。基础枚举、元数据、卷信息、读取和文件操作由 Rust/平台 API 提供。Git、进程工具、Cargo、Docker 走可选适配器。

替代：直接复制十几个文件会接管扫描、安全与平台维护；调用 ls/du/find 会重复遍历并增加文本解析和部署差异。独立二进制交付不要求“零编译期依赖”。

### D3 身份、观察与授权不混用

使用 `ResourceRef(server_id, scope_id, revision_id, node_id)`；无损定位与展示路径分离，卷/file ID 辅助对齐但不作为永久 ID。服务端通过授权范围解析引用，路径或 ID 本身不是能力令牌。文件枚举按时间窗口观察，记录覆盖而非假定原子快照。

替代：路径字符串永久主键会遇到重命名、编码碰撞及挂载替换；全盘可信路径会让远程查询变成权限放大器。

### D4 文件快照、证据批次与查询 revision 分离

DiskSnapshot 描述文件观察，CollectorRun 记录方法、版本、覆盖和时效，GraphRevision 固定组合。关系端点有类型、证据可支持或反驳；变化发布新批次。高频进程观察不要求全盘重扫，保护撤销必须来自合法来源。

替代：单一 mutable 当前图不能重放批准依据；一个永久布尔 safe 字段无法表达时效和未知。

### D5 图数据与控制数据采用不同生命周期

Rust store 管理本机 `diskgraph.sqlite`（索引）和 `diskgraph-control.sqlite`（scope/策略、计划、批准、操作、恢复、审计）。两库无跨库原子承诺：操作计划保存必要证据与指纹，文件副作用前持久意图，执行后再刷新图。图索引是可重建的；控制数据必须备份和保留。

PruneX 的 `prunex.sqlite` 只持有会话、偏好、UI 状态和经验证的批准来源投影，不能作为服务器执行权威。SQLite 文件留在各自机器；远程只通过服务访问。

替代：把恢复表放进可整体删除的索引会造成数据丢失；只放 PruneX 数据库会使独立服务不能恢复。

### D6 v1 保留与显式 v2

数据库、公共 API、规则和协议分别版本化。迁移前备份和空间预检，失败保留旧库；旧程序拒绝新 schema。v1 字段和五查询的结果保持兼容，legacy 证据保留原语义。v2 用字符串表达不透明 ID/字节数等跨语言风险数值，未知不填零。

替代：原地改变数字类型或将历史文本自动升格为已验证关系，都会破坏消费者与安全判断。

### D7 丰富能力与精简展示可以并存

按命令参考实现 29 个命令族和稳定 DTO。MCP 支持完整 read/manage/write 工具组，默认可采用精简 explore/status/explain/changes；权限主体可发现并使用已启用的精确工具。原生 UI 直接使用同一服务。自然语言由宿主转成显式模式/定位，DiskGraph 不新增模型路由。

替代：只有一个模糊工具会损失脚本精确性；默认把所有管理/写工具暴露给所有主体又增加误选和攻击面。默认展示策略用 A/B 验证，不作为永久能力上限。

### D8 三种传输，共用一个服务与权限入口

Rust rmcp 作为 stdio 和 Streamable HTTP 的优先 SDK，HTTP 宿主采用可挂载的服务适配。旧版 HTTP+SSE 在 `legacy-sse` 可选适配模块或受控网关实现，默认关闭但属于交付范围。SDK/协议版本在实施时锁定并做目标客户端矩阵；现代 SSE 流不计作旧版兼容证明。

替代：为每种传输独立开发工具会产生安全分叉；锁死旧 SDK 只为旧传输会阻碍现代安全修复。兼容适配不得绕过统一授权。

### D9 网络身份与目录权限分开

HTTP 采用 MCP HTTP 授权兼容的 OAuth 资源服务器模式，授权服务外置；开发/专用部署可使用已认证的加密隧道，但仍需明确主体到 scope 的映射，不把代理可伪造头当身份。默认 loopback、显式监听、Origin 校验、TLS/隧道、限流与大小预算。stdio 绑定本机用户身份并仍执行 scope/策略检查。

替代：仅要求知道服务 URL 或持有一个全盘共享密钥不能满足细粒度资源和写操作授权。

### D10 计划、批准、执行是不同权限动作

动作命令默认创建计划；plan show/validate 提供审阅，apply 接受精确 plan ID、可信批准引用和幂等键。批准由受信宿主确认通道或管理员预配置的限域策略产生，绑定摘要、身份、预算和期限；MCP 不提供可由同一智能体自由调用的自我批准工具。不能仅凭 TTY 或 `--yes` 判定人类同意。

执行重验文件身份、目录成员边界、策略版本、源目标权限和活动风险。平台安全句柄/不跟随链接/不覆盖操作用于降低竞态；不满足前提则拒绝。普通路径 canonicalize 后再修改不是充分防护。

替代：裸 delete(path) 或模型传 approved=true 无法证明用户批准的对象与实际操作一致。

### D11 诚实的恢复与空间语义

同卷移动优先原子不覆盖；跨卷复制采用 staging、校验、发布后再删除源，并有独立的批准范围。回收使用可验证的系统回收后端或卷内受控隔离区，保存恢复条目。restore 的冲突要求重新计划；purge 独立批准且不可恢复。不承诺跨资源原子回滚。

替代：回收失败退化为删除会突破授权；同卷隔离不是释放磁盘。分别报告逻辑处理、回收区保留和卷空闲实测，注明共享块/快照/并发活动影响。

### D12 可靠作业与保守失败

扫描按 scope 租约合并；修改按资源冲突序列化。持久操作意图先于副作用，幂等键绑定主体和计划，断线重连查询原 operation。崩溃后逐项对账，无法确认的不可逆状态转 needs_attention，禁止盲重试。取消仅阻止后续步骤。

替代：依赖 MCP 连接状态或内存会话来判定操作结果，会在超时、重连和服务重启时造成重复删除。

### D13 内容读取、去重与生态清理分别授权

默认元数据索引可有界解析允许的项目清单，不读取任意文档。read 有字节范围与类型限制；duplicates 先元数据候选，再内容授权下核验，避免占位下载。Git 检查不默认 fetch；Cargo/Docker 的清理用精确对象和专用策略，缺依赖不降级为直接删除目录。

替代：索引时全盘哈希会提高 I/O 和泄露成本；Docker 数据库/卷目录不能用通用文件大小推导安全清理。

### D14 平台宿主能力而非假想统一路径

macOS/Linux/Windows 使用原生路径适配；Android 由 Kotlin SAF/适用 MediaStore 提交 URI 观察；iOS 由 Swift 管理安全作用域和文档协调。FFI 批量交付事实、后台作业和可取消句柄。SQLite bundled 与 GRDB/Room 链接和生命周期逐目标验证。

替代：URI 转假路径、统一 root 权限、把编译通过称为真机支持均不可接受。移动端只开放 provider 真正支持且经过验收的操作。

### D15 私有发行与证据分层

内部发行物按平台包含校验和、版本、许可证和能力表；签名、公证与包管理分发在相应阶段验证。至少两个真实智能体宿主、三种传输、目标 OS 和移动真机分别验收。模型成本不套用参考项目数字。

## Risks / Trade-offs

- [全产品计划过大] → P0–P10 分阶段，阶段门禁独立；未验收平台/动作保持 disabled，不能一次性勾选全部任务。
- [上游路径已损失编码] → P0 源码与 fixture 核验，P1 先修复扫描链路或明确拒绝有损修改；必要时提出有证据的补丁，不静默复制。
- [低权限无法完整观察进程] → 输出 partial/unknown；P5 所需执行前安全检查不等待 P7 的富展示采集器，未知阻断操作。
- [SQLite 与文件系统无法同事务] → 控制记录先记意图、逐项状态与对账；图延迟刷新明确显示，不宣称全局 exactly-once。
- [索引、暂存或回收区挤满磁盘] → 分预算、满盘注入、预留控制日志空间；不足停止，禁止自动 purge 用户数据。
- [远程路径和内容泄露] → 主体授权过滤先于聚合、最小导出、日志脱敏、反向代理身份防伪。
- [旧协议增加维护面] → 隔离适配、独立兼容测试与能力开关；停用需另经需求变更，不把困难当作已支持。
- [两套文档与规格漂移] → specs 唯一需求源，命令编号 C01–C29 与任务/阶段矩阵校验，同步更新解释文档。

## Migration Plan

| 阶段 | 依赖 | 交付与退出门禁 |
| --- | --- | --- |
| P0 基线与契约 | 无 | 重跑既有测试；锁定 v1 fixture、能力模型、威胁模型和 ADR；不改用户数据 |
| P1 快照与存储 | P0 | 无损扫描、v2/控制库迁移、容量与恢复；旧接口回归 |
| P2 查询与 CLI | P1 | 项目证据、历史、有界查询和首批 CLI；未索引/未知不误导 |
| P3 本地智能体 | P2 | stdio、配置安装、私有 macOS 包、至少两个宿主与效率基线 |
| P4 远程只读服务 | P3 | Linux 服务、身份/范围、Streamable HTTP、旧 HTTP+SSE 兼容及断线测试 |
| P5 可恢复执行 | P4 | 计划/批准、控制日志、同卷移动/回收/恢复；安全与故障门禁 |
| P6 高风险及专业动作 | P5 | 跨卷复制/移动、purge、Cargo/Docker 精确计划；全链路测量 |
| P7 内容与桌面增强 | P2；写验证另需 P5/P6 | read/duplicates、应用/进程证据、watch、Windows 实机；不阻塞早期只读包 |
| P8 PruneX 嵌入 | P3；写 UI 另需 P5 | XCFramework、Swift/Kotlin/AgentScope、Rust 双库与 PruneX 业务库共存、受信批准通道 |
| P9 移动受限能力 | P8 | Android NDK/AAR/SAF、iOS 文档真机；P0 即预留 URI 类型，不等到此时改模型 |
| P10 完整私有交付 | P6/P7/P9 | 29 命令矩阵、三传输、平台安全门禁、性能/效率、升级/回退与支持矩阵 |

P4 将 Linux 服务器验证前置，是为了覆盖新确认的远程清理场景；移动契约仍从 P0 设计。P7/P8 可在各自依赖满足后并行实施，不等于授权当前启动多智能体或执行代码。

每个阶段先在专用 fixture/测试卷实施，随后只读试运行，再显式启用已验收动作。升级先备份两类数据库并冻结写作业；图库失败可恢复备份，控制库恢复必须先与文件系统对账，不能把回退数据库当成撤销已发生操作。旧版不能读取新库时使用备份实例或只读导出，不对正式库自动降级。

## Open Questions

- 已决定保留三种传输；实施时选择的 rmcp 精确版本、旧版适配依赖和客户端版本矩阵通过 P0/P4 spike 固定，不改变范围与授权要求。
- 采集器 TTL、默认索引/回收预算和查询时限由 P2/P7 的代表性基准校准；必须有上限，具体数值不作为未经实测的性能承诺。
- macOS 签名身份、Windows 签名资源及移动测试设备在打包阶段选定；缺少它们时只标对应验收未完成，不将编译替代实机证据。


### D13 2026-10-01 安全与性能加固

SC-06、CT-05、Q-08、RT-06/07、OP-10、ST-05 为本轮验收依据。请求上下文与共享 Engine 分离，token 能力只缩小实时数据库授权；revision 归属由持久记录确定。读连接独立，写连接串行；Unicode 规范化字段及搜索 keyset 游标保持既有语义。有序历史 merge 与按层树返回明确局部结果，SQL 超时也不得报告完整统计。

扫描保持 vendor pin，以既有 progress/cancel 每 20 ms 合作止损；staging 按实际编码计费、批次写入、SQL 发布。30 秒租约/5 秒续租及递增 fencing 贯穿写阶段。runner 严格认领，同名 owner 不复用尚未过期执行；取消标志按认领代次隔离，过期 owner 不能取消或清理新代次状态。目录/文件句柄与原子 no-replace 发布约束操作，复制在独占 staging 校验摘要与元数据，验证条件不足即拒绝。危险对外文件工具保持关闭。

取舍与限制：扫描仍先收集完整上游树，不承诺流式扫描或严格内存上限；规范化字段/索引及实时门禁增加扫描和物理数据库成本，收益主要是窄读和并发查询。旧操作引用无法精确到 revision 时保留整个 scope，逻辑 prune 不等于 SQLite 文件压缩。Linux/Windows、移动 provider 与真实宿主能力不以 macOS 夹具测试替代。完整证据见仓库双语加固记录及 benchmark JSON。

图库 schema 9 为不可变快照增加节点总数、目录总数/未知数、按不同 subtree_bytes 的降序累计计数。save、可信 publish 与 staging publish 在原发布事务内共用 SQL 聚合；v8 升级在一致性备份后事务回填，删除随 snapshot 外键级联。snapshot 的 writer 标记与 INSERT trigger 拒绝升级后仍打开的旧 writer；查询发现计数元数据缺失时 fail-closed，不能将缺失当成空快照。任意 minimum 的树计数仍包含未知大小节点的现有数值字段，children 的 known/unknown 谓词与 partial index 共用旧 JSON fallback。计数为索引探针；显式 OFFSET 页仍需 O(offset+page) 跳读。聚合与额外排序索引增加 O(N) 存储及发布/迁移耗时，并延长发布的控制库 fencing 锁；release 数据记录完整发布阶段而非精确持锁增量，1 ms 采样空间峰值不是上界。已有授权按完整 principal/permission/scope/policy_version 键只读返回，新增授权仍 INSERT ON CONFLICT；撤权 epoch 语义不变，避免只读 CLI 初始化重复授权写入。

### D14 Windows 本地普通文件内容句柄

15.11 保留 read/digest 的 wire 字段与 Unix 行为，将 Windows 获取步骤放入私有平台模块。路径规划只接受本地绝对 drive 路径与注册根下的原始组件，不用 canonicalize 跟随客户端路径；ADS、父级跳转、UNC/设备命名空间和过长路径明确拒绝。先持有 drive 根，再用 NtCreateFile 的 RootDirectory 逐个组件打开，禁止 reparse，并保留目录句柄直到检查结束。

当前线程通过 RAII 暴露占位属性，先只请求 FILE_READ_ATTRIBUTES，再检查文件类型及 reparse/offline/recall 属性，最后才对同一被持有父目录下的普通文件请求 FILE_READ_DATA。属性访问不冻结新 writer，获取期间的改变必须在申请数据前以 Conflict 拒绝；读取时的数据句柄仅共享读取，常规 writer/删除冲突。完整 128 位文件 ID、64 位卷 ID、长度及原生写入/变更时间检查结果是否稳定，不能用截断 ID 或路径重开代替；100ns 是表示单位，不保证各文件系统的实际精度或单调版本。局部模块分别拥有路径规划、原生状态、目录租约和跨平台内容准备，入口不新增业务类型堆积。

共享限制不是对内核/filter 或已有 writable mapping 的普遍冻结；版本检查也不是原子内容快照。FILE_OPEN_NO_RECALL 约束打开步骤，不能据此保证任意真实云 provider 的后续读取无下载。线程模式只覆盖调用线程，不能代表上游 scanner 的所有 worker；动态解析该可选 API，缺导出时返回 Unsupported，避免新增静态导入导致旧宿主 loader 失败。取消/期限在同步获取后及块之间检查，不抢占内核打开/读取。Windows 普通文件与属性/junction 夹具必须在实际原生 CI 通过，真实 provider、Linux 占位保护、写操作保真及移动端继续由 15.2–15.6 单独验收。

### D16 Engine 源码与状态边界

15.12 按 RT-09 整改整个 Engine crate：入口只声明及明确导出，对象含私有记录/trait/alias 每文件一个，生产文件少于 500 行，真实行为按 scope、policy、authorization、jobs、scan execution、retention、revision/history/tree、capacity 等责任组织。保留唯一 Engine 的连接、Mutex、取消/进度状态；既有 public content/live_evidence/verify 路径通过声明与重导出保留，对应类型、真实实现及测试分别归档。原生实现标注实际 Rust 来源，不虚构 Java 对应；每个 pub 方法说明中文用途、参数与返回。

结构迁移不抽新的 Service/Repository owner，不统一不同语义的时钟，也不添加授权或改变可信兼容入口的错误行为。graph→control 持锁顺序、已有 control guard 上的 require_with_control、发布后 collector 的独立结果、每 fence 的新取消 Arc、keeper/ProgressGuard 作用域及 ptr_eq 清理按原方法体保留。必要字段/helper 限于 pub(super) 或同等祖先范围的原 crate 协作权限；子模块迁移后的 crate 内重导出不得新增外部可调用通道。

先以 AST 门禁复现入口/多类型/行数/文档违例，再迁移并复核公开路径、类型/签名/Default/序列化及方法体。结构检查不证明并发授权安全；原行为测试、release 协议与同 SHA 三桌面系统/宿主/制品门禁仍必须实际运行。完成该增量不能替代原生写、真实 provider、移动和生产环境验收。

### D17 实时探针的原生身份、覆盖与执行边界

EV-06 的占用证据首先保留原生路径身份：按 NUL 字段的字节标签解析，不对首字符做 UTF-8 字节切割；查询键保留原生路径字节，只有可确认的 ASCII 显示字段才精确匹配；lsof 的 NUL 模式仍可能转义名称，带反斜杠/caret、控制/非 ASCII 字节或 deleted 标记的字段不猜测解码/归一，保持 partial/unknown。其他平台无法无损表示查询键时拒绝。格式、PID、上下文或终止符不足时保留 unknown，不返回完整空样本。可见句柄本身是正向观察，但外部 lsof 成功退出不能证明系统权限范围或 PID 启动身份，因此普通兼容入口报告 partial。空请求没有观察对象，继续保留原有空请求行为。

解析与覆盖子项不等于 EC-04 的执行边界完成。后续 runner 必须同时排空 stdout/stderr，共用绝对期限、累计输出与取消预算，在全部失败路径清理本次采样资源。Unix 进程组仅约束未自行脱离的后代；Windows 必须在执行前建立 Job/继承句柄边界，不能以 spawn 后分配补偿竞态。异步 I/O 的取消与最终释放需要对应原生环境验收，不宣称内核 I/O 的严格墙钟上界。

Git 尚无 HEAD 时仍需报告实际修改，预算/I/O/格式失败不得被解释为无 HEAD/upstream。仓库的 clean/process filter、fsmonitor 等可以让 status 启动外部程序；仅使用固定 Git 子命令或在执行前读取一次配置不能解决竞态。实现须采用受限不可变配置边界或显式拒绝，并以隔离哨兵证明不执行外部程序/不联网/不产生可选索引写入。支持的 Git 版本、特殊仓库配置和未验收平台分别记录，不能借用本机新版 Git 的结果。

### D18 探针的共享资源执行器

15.13b 在 Engine 内建立私有执行器，避免 Engine→Ops→Engine 循环依赖；原有两个采样函数作为默认 15 秒/1 MiB 兼容入口，新增带期限、累计输出与取消配置的入口。每次采样持有独立预算，Git 多个命令复用同一绝对期限和两管道累计字节；任何资源失败立即终止，不由 `.ok()` 降级为无 HEAD/upstream。默认值不是生产 SLA，缓冲和输出有界不代表全部探针内存或内核 I/O 可被硬抢占。

Unix 使用本次子进程的独立进程组与非阻塞管道，轮询两管道及进程状态；WNOWAIT 保留 leader 身份至清理和回收完成。仍存活的 leader 即使离开原组也按自有 PID 终止，不追随新 PGID；主动 setsid 逃离的后代不在组约束内。Darwin 对仅 zombie 的组返回 EPERM 时，最多 50 ms 重试，最终要求全部 SZOMB、leader 父身份及两次完整成员集合一致，未知/变化仍失败；退出过渡本身不等于已退出。Windows 10+ 使用原生 CreateProcessW 的 JOB_LIST/HANDLE_LIST 原子绑定 KILL_ON_JOB_CLOSE Job 和三个标准句柄，禁用 breakaway；不以运行后 Assign 或缺能力 fallback 代替。父读端是自有 overlapped named pipe，每条固定缓冲与 OVERLAPPED 地址稳定，CancelIoEx 后等待最终完成再释放。终止 Job 后保留句柄，最多观察 1 秒以确认 ActiveProcesses 为零；查询失败或实际非零超期明确返回清理失败；用户 HANDLE 本身不是失败原因，不假设它必然维持非零计数。关闭 Job 提供 KILL 后备；leader 等待及自有 pending I/O 最终完成不受这一秒硬限，安全清理可能超过协作采样期限；平台行为必须在对应 CI 执行真实后代/管道与并发回归。

成功要求正常进程退出状态、stdout/stderr EOF 和末段期限/取消同时有效；信号死亡或 Windows 高位异常状态不得降级成无 HEAD/upstream。恰好耗尽字节预算仍以固定小缓冲检查 EOF，多一个字节即拒绝完整结果。stderr 同样扣预算，不因解析器未消费它而忽略。Windows 零字节成功读取不等于 EOF，意外 abort 是业务失败，仅本 owner 的清理取消可作已终止 I/O。显式清理必须保留主错误及次级清理诊断，Drop 仅作兜底。Windows 兼容入口明确要求绝对项目目录和受信 .exe，不能宣称全部旧平台行为相同。该资源子项不关闭 Git 配置、执行 filter、离线/只读保证、unborn 或引用格式语义；父项 15.13 和 8.7 继续单独验收。

### D19 — Git 可观察语义

15.13c 使用同一受限执行器与整次预算，但不声称已隔离仓库配置。采样要求 Git 2.46 或更新版本，利用 show-ref --exists 的 0/2/1 明确区分存在、缺失和查询错误；不兼容工具明确拒绝。symbolic-ref --no-recurse 保留 HEAD 的直接目标，只有直接本地分支明确不存在时才表示 unborn；已存在的悬空 symbolic branch、损坏引用、非 commit 对象、执行错误与格式错误不能降级。状态采用 porcelain v1 -z 与 untracked-files=all，按原生 NUL 记录计数，rename/copy 的第二路径不额外计数。

stash list 即使成功也可能跳过坏 reflog 或缺失旧 commit。stash 存在时完整计数仅支持 Git 明确报告的 files 引用后端，由 rev-parse --path-format=absolute --git-common-dir 定位 Git metadata common 根，再追加固定 logs/refs/stash，保留 linked worktree 共享语义。不能直接 canonicalize 完整日志路径，否则 leaf/日志目录链接已在 nofollow 之前被解析；metadata common 根自身由 Git 定位并不构成 scope 或配置隔离认证。读取在同一整次期限/取消/累计字节预算内完成，拒绝固定日志路径中的链接/reparse/非普通文件及无法无损解释的路径。校验固定 old/new OID 前缀与 committer/timestamp/timezone 格式，每个留存 new OID 必须是 commit；记录反序与固定 %H 列表逐条核对，不以集合去重。drop、无 rewrite 删除及 expire 的合法语义不要求 old/new 连续、old 对象仍存在或消息非空，有效 tip 与合法空日志或明确缺日志可计零。末段复核 tip 与日志身份/版本及完整内容，变化拒绝；reftable 或未确认日志明确拒绝，不从工具空成功推断完整。该读取及复核不提供原子 Git 快照或配置隔离保证。

upstream 从当前分支的 for-each-ref 固定 NUL 格式解析；没有可解析跟踪映射或本地跟踪引用缺失都保持 unknown，并分别说明原因，不能声称已推送。已存在但损坏/缺对象的引用报错。比较只传校验后的完整 OID 范围，提交数量必须是完整十进制 u64，格式/溢出错误传播。采样末段重新读取 HEAD/分支，变化时拒绝混合结果；这只是检测，不是原子 Git 快照。特殊 Git 配置、filter、懒取对象、可选 index 写入及不可变配置边界继续由 8.7/15.13 验收，语义子项不能关闭完整生产门禁。

### D20 — Git 私有配置和元数据视图

15.13d 首先建立本次采样独占的私有 bootstrap 与预算，再进行准备。全部工具调用固定关闭 pager、懒取与 optional locks，清空继承的 Git/loader/trace 环境；解析复制配置时显式使用私有目录、空 system/global 文件及 `config --file --no-includes --null --list`，只把配置当数据。不能在原仓库调用 `rev-parse --shared-index-path`：真实夹具已经证明它会执行 fsmonitor、刷新 shared index 时间，即使 no-lazy-fetch/no-optional-locks 也无效。其他 metadata plumbing 的安全性限定到固定参数和格式，不把默认 for-each-ref 等会解析对象的形式泛化为安全。

工具只从绝对 PATH 目录解析一次并固定绝对程序路径，私有准备和源工作树采样不得重新选择程序。目录租约仅存活于单次 no-follow 捕获/枚举，历史记录保存身份、完整版本、祖先身份和名单；最终复核重新打开，避免目录数量累积 FD/HANDLE。原生名称名单未命中时必须查证实际路径，大小写或其他别名不能被报告为缺失；无法可靠映射时明确 Unsupported。准备和公开终态显式清理私有目录，主错误与次级清理诊断均保留，清理后再次核验预算，Drop 只兜底。

由原生有界读取捕获 `.git`/gitfile/commondir、config、index、HEAD、loose/packed refs、stash 日志、shallow 与必要属性/忽略文件，拒绝链接、非普通文件、不可核验版本和超限输入。linked worktree 的 HEAD/index 与 common 根的 refs/logs/objects 分别解析，私有视图不能保留指向原元数据的 commondir/config/index/refs/reflog。元数据身份、版本和字节在末段复核，变化报错；这不是原子仓库快照。复制 index 后恢复并读回确认原高精度 mtime；较新的复制时间会改变 Git racy-index 内容复核，已在同长度改写并恢复文件 mtime 的夹具中真实产生假 clean。零时间会关闭该判定，不能作为安全备选；精度无法保真必须拒绝或另验保守重验。

配置生成采取类型校验的 allowlist，保留 filemode、ignorecase、symlinks、autocrlf/eol、必要属性/忽略/rename 与 branch/upstream 映射等状态语义；不复制外部命令、URL、helper、hook 或 pager。不能简单关闭 filter 后把透传内容当正常状态：初版可明确拒绝外部 clean/process driver；支持未使用 driver 时须以无执行命令的 required 失败兜底，另验属性替换/宏展开竞态。include/includeIf、split/sparse index、gitlink 子模块、reftable、partial/promisor、replace/grafts 和未知扩展在没有等价实现时明确 Unsupported；这些支持边界不表示永远删除相关能力，也不能把所有 Git 仓库关闭后宣称完成。普通 files/SHA-1/SHA-256/full-index、linked worktree 与 shallow 的兼容性分别通过实际差分验收。

执行时只使用私有 git-dir/index/refs/shallow/config，原 worktree 作为实时数据读取。源对象库只读借用可以降低整 pack 复制成本，但 alternate 会递归读取源 info/alternates；若采用该方案，不能宣称对象访问范围仅限原 ODB、对象快照不可变或对象输入严格有界。严格文件访问范围需要额外的扁平 ODB 视图或原生隔离能力，不能由一次预检查证明。接上源对象库后禁止写对象 plumbing，避免源对象时间刷新；所有来源元数据和对象时间、lock 文件以只读哨兵核验。

源 ODB 借用已在隔离公共库夹具复现跨目录及未计费输入，不再作为 D20 的最终实现。后续执行视图改为有界私有扁平 ODB：原生 no-follow 捕获安全十六进制 loose 路径及普通配对 `.pack/.idx`，保留 SHA-1/SHA-256、linked 与 shallow 的必要语义，明确拒绝 alternates/promisor 及不可核验路径，不复制可引入额外路径的 info 文件。源对象捕获、复制和最终内容/身份/版本复核与既有元数据共用 64 MiB/32k 累计额度及整次期限/取消；私有副本走同一 128 MiB 对象报告分配、64 MiB 卷余量 owner。Git 不持有源 ODB 目录引用，准备后的新增 alternate 因而不能成为工具输入。不得硬链接复制：nlink/ctime 会改变源对象。原始数据读取、复制和内容比较随捕获对象字节线性增加，另有目录枚举、排序、原生路径解析和记账成本；累计预算包括两次读取，大型 pack 会明确超限；既有公开 ProbeLimits 签名不变，不宣称适用于任意大小仓库、原子快照或严格 RSS。

准备、采样和终态复核共用唯一绝对期限及取消，分别明确管道/原始输入累计字节、元数据条目和临时容量上限；限制不是全部 Git RSS 或内核调度上界。先把已复现的 fsmonitor/filter/懒取/shared-index/racy-index 探针转为回归，再实现视图、运行普通状态差分与失败清理，独立复审及同源码三桌面原生 CI 后验收。15.13c 的语义测试和已有只读包 CI 都不能替代该边界，父项 15.13、8.7 与全平台其余能力继续未完成。

初始实现差距（011fe7f 阶段，不降低以上验收要求）：宿主系统 config 的编译时路径由固定 `config --system --no-includes --show-origin --show-scope --null --get-regexp '^'` 首次发现，此次 Git 读取只有共享期限、取消和管道输出额度，没有读取前的原生原始输入/RSS 上限。发现后才对唯一原始来源有界捕获、解析副本和终态复核。64 MiB/32k 元数据额度当时仅约束累计输入与复制内容，没有实际分配磁盘空间或卷剩余容量门禁。后续以固定 printer 发现、原生捕获及独立容量账本收敛，实际验证状态保留在 tasks 与双语验收记录，不能把设计视为完成。单次路径解析仍临时占用 O(depth) 句柄，源 ODB 的递归 alternates 仍不是严格输入/访问范围上限；未执行的原生平台测试阻止勾选 15.13d。

后续实现依据：Git 2.46 的 `config --system --edit` 在 `GIT_CONFIG_NOSYSTEM=1`、空私有 global/repository 配置及固定 `GIT_EDITOR="printf '%s\\0'"` 下，仅计算并规范化目标路径，system 分支不创建/读取该文件，编辑器不回读内容。Git 通过自己的受信 shell 固定调用 printf，并以独立 argv 传目标，不拼接路径为代码。该内部发现接口须以 FIFO/include/缺失文件/空格及 shell 元字符路径真实验收，严格解析单条 NUL 路径，首次内容读取改为原生捕获；不得借用未知编辑器或放开源配置。此能力依赖受信 Git 发行包及其 shell，PATH 仅保留受信绝对目录；输出为宿主物理路径，终态重发现检测别名重定向，source 仓库元数据仍执行 no-follow。跨系统实际测试前不得泛称可用。

私有容量新增每样本默认 128 MiB 对象原生报告分配额度及 64 MiB 卷可用余量，分别约束原始输入、对象分配和可用空间，不新增 ProbeLimits 公开字段。创建/写入前读取实际 available，写后增量更新叶及父目录分配（避免每次全树扫描），末段一次遍历核验所有已登记项；未知项/类型/身份/分配或原生容量查询失败拒绝。Unix blocks 与 Windows AllocationSize 只代表对象报告的分配，压缩/resident/sparse、日志及卷全局元数据未必归于对象；不能称严格总物理硬限、空间预留或外部抢占保护。

容量与内容版本分开复核：普通文件记录完整原生身份、长度和不含访问时间的高精度版本，只有 owner 写入或设置 index 时间后更新水位；Git 读取期间的同长度/同分配原地修改也必须拒绝。清理前确认账本的根身份，拒绝删除替换来的陌生根并保留主/次诊断。这是可信宿主临时路径上的变化检测，身份预查与按路径删除并非原子操作；旧兼容缺失路径只能表示该路径已消失，不能证明被重命名到其他位置的 owner 对象已清除，也不承诺抵抗同权限进程的全部交换竞态。

Windows 工具表示与原生身份路径分开：Git for Windows 2.55.0.windows.5 的真实 CI 在配置读取时拒绝 `\\?\C:\…`，`worktree add` 也报告 `//?/C:/…` 为无效路径。Git 工具环境、工作目录、`config --file`、私有属性/忽略配置及 alternates 经同一词法适配；仅对本地 drive 的无歧义名称移除 verbatim 前缀，原生 no-follow/身份路径保持不变。拒绝 UNC/设备命名空间、点步、ADS、尾点/尾空格、保留设备名、无法表示的编码及普通工具路径超过 259 UTF-16 单元；不关闭 protectNTFS、不继承宿主 Git 配置。原生夹具比较适配前后同一文件身份、实际配置与 printer 输出，普通/linked 仓库回归通过后才有 Windows 支持证据。词法验证本机通过不等于 Windows 原生验收。

宿主 shell PATH 是搜索目录列表，不替代实际执行路径的身份边界。固定 Git 程序仍先从受信绝对目录解析一次；shell PATH 仅捕获可安全表示的绝对项，不把无关且不可表示的项带入 child 或据此拒绝已定位程序。每项仍执行整次取消/期限检查；SystemRoot、worktree 和私有目录/文件依旧严格拒绝不可保真输入，缺少实际 shell 时传播启动错误。构造与 spawn 的分段诊断不输出整个宿主环境；原生测试记录被排除的 PATH 路径与具体阶段，以区分列表兼容问题和实际身份拒绝。

源 metadata 哨兵的夹具准备必须在取水位前结束自己的后台写入。Git 2.55 的 commit 会启动自动维护，默认允许 detach；维护持有 `objects/maintenance.lock` 并在后台结束后删除它。夹具通过每次准备命令的固定 `-c maintenance.auto=false` 在首个 commit 前阻止此后台任务，Trace2 回归确认实际 commit 已执行且没有启动 maintenance child。此设置仅约束临时夹具，不改变生产采样策略；完整路径名单、字节、mtime 与数量断言继续保留，原生 CI 仍须验证 SHA256 语义及源哨兵。

Git for Windows v2.55.0.windows.5 的 `core.fscache` 属于明确的布尔配置，原值依宿主/仓库的捕获顺序回放。官方 `compat/mingw.c` 通过布尔解析器处理，`compat/win32/fscache.c` 只实现每个 Git 子进程内的只读目录/stat 缓存，不调用配置程序或写回源元数据；启用期间不会反映其后工作树变化。支持此字段不得泛放未知 core 字段、fsmonitor 或 driver，不构成原子状态或严格 RSS 保证。类型/覆盖顺序、真实 dirty/stash/upstream 与跨次独立采样的缓存释放均须回归；本机 Git 能证明字段回放，缓存行为仍需 Windows 原生验收。

### D21 Ops 源码与副作用边界

OP-14 延续已经完成的 Store 与 Engine 结构约束，整改完整 Ops crate，而不只移动入口中的测试。当前入口 2406 行、specialist 生产段约 641 行，docker 虽生产段不足 500 行却包含多个独立对象；三者一起纳入源结构门禁。每个真实对象独立文件，函数按授权、摘要、路径编码、实时重验、容量、操作查询和刷新职责组织；specialist/docker 保留旧公开模块路径和精确根重导出。

PlanBuilder 保持原有全部行为与批准顺序。Executor 仍是唯一执行状态 owner，其既有方法按 apply、validation、perform、transfer、paths 分为实际 impl 模块，不新增 facade service、线程、锁、事务或接口。CrossVolumeCopy 保持互斥平台实现、两个 Mutex、句柄和批准 Metadata 所有权，stage/publish/discard 顺序不变，不通过自动 Drop 改变 NeedsAttention 或失败清理时序。相邻私有模块所需的可见性只扩大至 crate 内，不变为公共 API。

实现先记录公开导出、类型和方法体基线，再使 AST 结构门禁真实失败，拆分后对照规范化源码与已有安全/并发/恢复测试。中文注释使用实际 Rust 来源，不虚构 Java 类型；不借本次整理改造 specialist runner 或启用危险工具。全 workspace 与同 SHA 原生 CI 验收后才能完成结构子项，其余平台写操作、provider 和移动端门禁保持独立。
