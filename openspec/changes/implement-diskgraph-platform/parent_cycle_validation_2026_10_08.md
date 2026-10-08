# 父链循环拒绝与非递归验证

## 已复现缺陷

旧 graph_validation 只确认 ID唯一、父ID存在和唯一匹配根，未确认父链无环。隔离图库创建合法原版本后，额外自环节点仍被旧发布实际接受（Ok），目标回归失败。测试的最初方法名编译错误已纠正；归档红灯是实际行为失败。

## 实现

既有节点ID集合改为ID→索引映射，复用父ID验证与证据ID核验。增加每节点1字节灰/黑状态，沿父链最多进入和完成一次；没有递归或逐节点重扫整条祖先链。有限无环父链配合原有唯一根检查保证连接到同一根。

兼容发布和带实际server/scope的staging发布均拒绝自环、双节点环，先前latest保持不变。逆序排列的20,000层合法树通过。现有显示别名与无损定位规则不变。

## 本地验证

publication_regressions 7通过，native_locator_tests 10通过，job_publication_check_tests 5通过；Store Clippy -D warnings、格式检查及OpenSpec strict通过。原始日志见docs/benchmarks/parent_cycle_validation_2026_10_08。Store完整测试361通过、9忽略，忽略项不计验收。源码遍历为线性节点访问，哈希查找为期望常数成本；不将测试耗时称为全平台性能门禁。

## 独立 macOS 排查

隔离四进程共4,000次槽位认领探针包含openat、真实flock、记录写入、fsync、回读与释放，未复现ENOENT。它没有运行Engine启动或生产监督器，不证明CI问题已修复；原始脚本和结果单独归档于同目录。macOS启动问题继续开放。

当前远程5dc825d CI不含本修复，整体生产门禁保持未完成。
