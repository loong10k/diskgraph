# 扫描 staging 复用预算准入编码（2026-10-08）

对应 snapshot-storage 的 Reuse the admitted staging encoding 增量场景。

原 Engine 每节点先用 staging_observed_node_encoded_cost 编码 JSON、Unicode 搜索字段和原生观测以计费，Store 写入又再次编码。现 Engine 构造字段不可被外部替换的 PreparedStagingNode，直接移交原 QualifiedLocator，按同一载荷 encoded_cost 准入；当前配置批次写入使用已经编码的字段。取消、授权、lease/fencing、容量与每批事务仍沿原调用链，旧 append_staging_* 方法保持签名和校验，转入同一写入实现。

缓存限于当前 write_batch_nodes 批次；它会延长该批 JSON 和搜索字段的存活时间，不承诺严格 RSS 上限或无新增内存。Unix 旁表成本与编码沿用原实现，本次只消除公共节点载荷的重复编码。无 schema、wire 字段、SQLite 持久策略或 vendor 改动。

新增三个真实 SQL 回归验证：计费后改变原节点不能替换已准备载荷、字节成本与实际行字段一致、中途取消回滚节点和搜索、提交前取消回滚本任务且保留另一任务原记录。初次测试由于新增 API 不存在而编译失败，这不是已复现行为红灯，不作为性能缺陷运行证据。

最终本机 macOS ARM64：目标测试 3/3；Store lib 339 passed、5 ignored；Engine source_layout 6/6；Store/Engine all-target Clippy 和指定文件格式检查通过；OpenSpec strict 有效。Store 测试使用混合工作树，包含原来未提交的 reader_observation_tests 挂载，不能冒充干净提交测试。

尚未运行当前改动的 Linux/Windows 原生扫描或 release 成对性能对比；Windows 200k 的原 300 秒完整门槛仍失败。消除重复编码的源码事实不等于整体耗时降低已量化。当前改动不能关闭生产就绪验收，也未勾选相关 tasks。

证据：docs/benchmarks/prepared_staging_2026_10_08/，含当前源码摘要与原始压缩日志。

## Windows 条件导入构建修复

bda24e4 的 Windows Kotlin 原生作业 113240050571 在实际 Rust 构建中失败：新 staging 路径不再需要 Windows 分支的 WindowsObservationGap 类型注解，但它仍被无条件导入，-D warnings 将未使用导入视为错误。修复仅为该导入增加 cfg(not(windows))，不放宽警告、预算或业务测试。原生 RED 日志保留于 docs/benchmarks/windows_staging_import_bda24e4_2026_10_08/。本机 Engine all-target Clippy、架构检查 6/6 和格式通过；当前新源码 Windows GREEN 仍须下一轮 CI，不能以 macOS 编译代替。

本次因为已证实候选的原生构建错误而更新提交并重启矩阵，不是因为观察超时而重启；旧候选剩余作业取消仍保留其原始失败，不作为完整验收结果。
