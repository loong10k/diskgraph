## Purpose

规定 DiskGraph 对文件系统的观察边界、资源身份、尺寸口径与扫描覆盖，使快照可用于可信查询和历史比较，同时明确平台差异、变化期间的不确定性和云端占位资源限制。

## ADDED Requirements

### Requirement: FS-01 Snapshot observation window
系统 SHALL 为快照记录 server/scope、根、可得卷或 provider 身份、扫描起止时间、选项、扫描器版本和逐范围覆盖；普通遍历仅声明 best_effort 一致性。

#### Scenario: Concurrent mutation
- **WHEN** 扫描期间文件变化或消失
- **THEN** 记录变化/失败范围，不声明原子全盘快照。

### Requirement: FS-02 Lossless locator and qualified identity
系统 SHALL 区分无损原生路径或 URI 定位、展示文本、快照内 ID 与辅助文件身份；inode、路径、PID 或内容哈希不得单独作为永久身份。

#### Scenario: Non UTF8 collision
- **WHEN** 两个原生名称展示为相同替换字符
- **THEN** 底层定位和节点保持可区分；无法无损处理时报告限制并禁止定位型修改。

#### Scenario: Malformed Windows native encoding
- **WHEN** Windows 原生定位的解码字节不是完整的 UTF-16 代码单元序列
- **THEN** 返回无效原生编码错误，不截断末尾字节或改用展示文本定位。

#### Scenario: Qualified native locator survives publication
- **WHEN** 当前进程的原生扫描产生无损定位，并完成暂存、事务发布及数据库重开
- **THEN** 按实际 revision 所属快照和节点 ID 可窄读同一原始字节、明确编码和自身修改时间；编码数据预算包含定位原始字节，失败发布不留下半份定位。

#### Scenario: Historical locator has no encoding provenance
- **WHEN** 旧快照只有展示定位，或原始定位没有明确的原生编码来源
- **THEN** 迁移不猜测回填原始字节或平台；需要可靠原生定位的入口返回 unavailable/unsupported 并提示重新索引，原有可信展示查询保持兼容。

#### Scenario: Foreign or unknown native encoding
- **WHEN** 持久定位标记了其他平台或不支持的编码，或 kind、编码和原始字段损坏/不一致
- **THEN** 在文件访问之前明确拒绝；不得依赖当前宿主猜测字节含义、截断 UTF-16 或从展示文本恢复路径。

#### Scenario: Oversized stored locator
- **WHEN** 单行或累计持久定位字段超过请求的原始字节/节点额度
- **THEN** 在从 SQLite 借用字段分配拥有对象之前拒绝，空结果及终态仍检查共享期限；不为精确节点读取整棵树。

#### Scenario: Reused inode
- **WHEN** 历史快照的文件 ID 被新文件复用
- **THEN** 不能仅凭文件 ID 宣称它是原文件。

#### Scenario: Windows native identity cannot fit the compatibility field
- **WHEN** Windows 的原生 128 位 file ID 无法无损放入已有 64 位兼容字段，或读取句柄身份失败
- **THEN** 身份返回 unknown，不截断、散列或采用路径冒充；NTFS 可表示的身份以实际卷序列号限定，并通过硬链接与替换夹具验证。

#### Scenario: Full Windows native observation persists independently
- **WHEN** 一个安全属性句柄提供卷序列号、完整 128 位 file ID、长度及 creation/last-write/change 原生时间
- **THEN** 保存完整位模式、100ns 表示与独立捕获窗口，经批次暂存、原子发布和重开保持一致；不能投影到旧 u64 时兼容身份仍 unknown，不截断/散列，不把访问时间纳入版本比较。

#### Scenario: Windows attributes remain bound to the held scope root
- **WHEN** 文件树遍历后进行补充属性采样，或父/根目录发生替换、重解析
- **THEN** 使用遍历开始前固定且保留的原生根句柄逐组件解析；父组件和根拒绝重解析及跨卷，末组件只能从安全父句柄打开对象本身属性，不能申请正文权限或回退到完整路径重开；旧路径扫描不因此被称为原生句柄遍历。

#### Scenario: Attribute leases do not freeze the Windows namespace
- **WHEN** 属性句柄仍指向原目录，但注册根或其祖先被实际重命名、删除或替换为其他绑定
- **THEN** 重新验证当前 drive 锚及各保留父句柄下的单组件名称绑定，比较完整卷/128 位身份和目录安全状态；绑定缺失、身份改变、重解析或跨卷返回 conflict，采样和发布均被拒绝
- **AND** 目录自身修改时间变化保持允许；不为冻结名称额外申请正文/枚举/删除权限，不声称属性共享标记保证重命名失败，也不将有限复核称为原子文件系统快照

#### Scenario: Linux scan observation rejects ancestor rebinding
- **WHEN** Linux 扫描的补充原生观测完成后、发布之前，注册根的祖先目录被替换，即使原根和文件被移回相同展示路径且最终 inode 未变
- **THEN** 使用遍历开始前固定的原生锚与逐组件名称绑定验证祖先和根；发现绑定变化时返回 conflict，任务失败并清理本次暂存，不发布 revision 或推进 latest
- **AND** 祖先目录中无关 sibling 的增删保持允许，不用目录修改时间冒充身份，不声称该复核使旧路径遍历成为原子快照或句柄遍历

#### Scenario: Native observation and old tree projection are not atomic
- **WHEN** 补充观测与旧树的可比辅助身份、类型或明确可比的尺寸矛盾，或同一句柄前后版本变化
- **THEN** 记录固定类型的 changed/unknown 原因，不用零值、旧秒时间或路径补出完整身份；不可比较的 allocated/dedup/聚合尺寸明确标为未对齐，不覆盖 v1 尺寸或认定内容相同。

#### Scenario: Full native observation bounded read and migration
- **WHEN** 按 revision/node 读取完整原生观测，或从 v11 数据库升级
- **THEN** 在 SQLite 借用字段拥有/解码前计费，使用实际 revision 归属和首末实时授权、同一绝对期限；历史全空字段为未捕获，版本/长度/类型不一致明确失败，迁移一致性备份不猜测回填，旧已打开 writer 不得静默发布缺少新协议的数据。

### Requirement: FS-03 Size semantics
系统 SHALL 分别表达 apparent、allocated、direct/subtree 尺寸与未知数量，区分自身 mtime 和子树聚合时间；硬链接统计口径明确且不把观察尺寸当作可释放保证。

#### Scenario: Unknown allocation
- **WHEN** provider 不提供实际分配大小
- **THEN** 返回 unknown 而不是零或表观大小。

#### Scenario: Hard link group
- **WHEN** 两个路径引用同一已识别硬链接对象
- **THEN** 按声明口径避免重复统计，并说明删除单一路径不保证释放。

### Requirement: FS-04 Coverage and exclusions
系统 SHALL 记录权限拒绝、深度上限、取消和 provider 限制；缓存、隐藏目录、target 和 node_modules 不得因代码索引习惯被默认忽略；自身索引/控制存储应排除。

#### Scenario: Unreadable subtree
- **WHEN** 一部分目录不可读
- **THEN** 快照标 partial 并返回对应原因，而不是零文件。

#### Scenario: Build outputs present
- **WHEN** 授权根内有 target 和隐藏缓存
- **THEN** 按显式扫描策略纳入观察。

### Requirement: FS-05 No hydration and link loops
默认扫描 SHALL 不跟随链接越界、不触发云端占位文件内容下载、不遍历循环，并根据明确设置处理跨文件系统边界。

#### Scenario: Cloud placeholder
- **WHEN** 扫描遇到仅云端内容的文件
- **THEN** 记录占位与未知元数据，不读取正文触发下载。

### Requirement: FS-06 Incremental invalidation
系统 SHALL 将文件事件视为重查提示，变化失效相关证据；事件丢失、休眠、挂载或权限变化触发受影响范围重扫，失败发布不得推进最新完成版本。

#### Scenario: Watcher overflow
- **WHEN** 事件队列溢出
- **THEN** 标记范围需重扫，不继续声称索引新鲜。

#### Scenario: Partial comparison
- **WHEN** 旧快照有文件而新扫描未获该目录权限
- **THEN** 不能把该文件确定报告为已删除。
