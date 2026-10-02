## Purpose

在单独内容授权与资源预算下支持远程及本地的有限文件读取和重复内容核验，避免因调查目录而读取全盘文档、下载云端占位文件或把重复线索直接转化为删除授权。

## ADDED Requirements

### Requirement: CT-01 Bounded explicit reads
read SHALL 要求独立内容权限、精确资源引用与字节/范围预算，返回实际读取范围和截断信息；默认拒绝设备、管道、socket 等非普通内容对象。

#### Scenario: Huge file
- **WHEN** 请求读取大文件且只授权前一段
- **THEN** 只读允许的范围，不先整文件载入内存。

#### Scenario: Content not granted
- **WHEN** 只具备元数据权限的主体请求正文
- **THEN** 拒绝且不把正文写入响应或日志。

### Requirement: CT-02 No implicit remote hydration
内容检查 SHALL 不默认下载云端占位文件、不跟随链接读取越界目标；文件在检查期间变化时结果必须标记失效。

#### Scenario: Placeholder hash
- **WHEN** 重复检查遇到云端占位文件
- **THEN** 标记 skipped/unsupported，不触发下载来计算哈希。

#### Scenario: Windows native content acquisition
- **WHEN** an authorized caller reads a local ordinary Windows file
- **THEN** resolve single name components relative to retained directory handles, expose native placeholder attributes on the calling thread, acquire only attributes before approving data access, and reject reparse/offline/recall objects before reading data. Acquisition changes must return conflict before data access; once acquired, the data handle denies ordinary write/delete sharing for the read lifetime. Retain directory/attribute leases and full native identity/size/version checks, restore the calling thread's prior compatibility mode, and refuse a missing mode API explicitly. Native timestamp units are not a guaranteed atomic/monotonic content version. Real cloud-provider no-hydration acceptance remains separate from local NTFS and attribute fixtures.

### Requirement: CT-03 Staged duplicate confirmation
duplicates SHALL 先基于元数据生成疑似组，明确授权后才执行有预算的内容哈希与确认；硬链接、共享块和独立重复副本分别说明，结果不能自动触发删除。

#### Scenario: Equal size different bytes
- **WHEN** 两文件同名同大小但内容不同
- **THEN** 只能作为疑似，核验后不得报告 confirmed duplicate。

#### Scenario: File changes during confirmation
- **WHEN** 核验中的任一文件被写入
- **THEN** 丢弃该次确认或报告 unstable，不能沿用旧哈希授权删除。

### Requirement: CT-04 Minimized retention
内容读取和去重 SHALL 默认不持久保存完整正文；摘要、哈希、日志与导出遵守授权和容量/保留策略。

#### Scenario: Remote model response
- **WHEN** 导出策略允许元数据但禁止内容
- **THEN** 不得借 explain、日志或错误消息泄露读取内容。

### Requirement: CT-05 Enforced digest budget
内容摘要 SHALL 每次读取扣除字节预算，验证打开句柄的资源身份与稳定性；无法确认完整内容时不得返回 confirmed 摘要。

#### Scenario: One byte budget
- **WHEN** 1024 字节文件请求 max_bytes=1
- **THEN** 最多读取 1 字节并返回未确认，摘要为空。

#### Scenario: Parent replacement or in-place change
- **WHEN** 父路径被替换为链接或同 inode 被原地修改
- **THEN** 拒绝范围逃逸或返回 unstable，不确认混合版本。

#### Scenario: Verification pair fails on one side
- **WHEN** a content comparison reaches a readable left file but the right side fails or loses authorization
- **THEN** the attempt still consumes its file allowance, all bytes already read remain in the cost report, and the pair stays unverified.

#### Scenario: Verification deadline or cumulative budget
- **WHEN** a verification pass exhausts its cumulative byte allowance or deadline
- **THEN** no new reads begin, in-progress hashing stops between chunks, and incomplete hashes cannot be promoted to confirmed results.

#### Scenario: Authorization wait consumes the deadline
- **WHEN** native acquisition or current authorization waits until after the digest deadline
- **THEN** recheck the deadline after authorization and before any new read; return an unconfirmed deadline result with the actual bytes already read, including zero bytes for an empty file.

#### Scenario: Terminal digest confirmation
- **WHEN** EOF, an exact byte limit, or final identity observation completes a digest
- **THEN** recheck current authorization, cancellation and the deadline before confirming, including empty files; a stop voids the digest without erasing read costs. Synchronous waits are not hard-preempted, but a late result cannot be confirmed.

#### Scenario: Windows native ordinary file regression
- **WHEN** a Windows NTFS fixture is inspected under explicit content grants
- **THEN** bounded ranges and complete digests work, one-byte digests remain unconfirmed, cancellation/deadline and revocation return no confirmed content, and alternate data streams and namespace escapes are refused. A writer present when acquiring data conflicts; mutation during the attribute-only acquisition is rejected as conflict without body/confirmed digest; a writer opened while reading conflicts with the data handle, and the retained parents prevent replacement. Cancellation/deadline are cooperative between native I/O operations, not hard preemption of a synchronous open or read.
