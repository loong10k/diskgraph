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

#### Scenario: Reused inode
- **WHEN** 历史快照的文件 ID 被新文件复用
- **THEN** 不能仅凭文件 ID 宣称它是原文件。

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
