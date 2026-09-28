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
