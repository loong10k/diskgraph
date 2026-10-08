# Unix 观测批次 SQL 编译复用

本次最初怀疑 JSON ID 目标校验全表扫描，但当前源码已有 scan_staging_by_job_node_id 表达式索引，真实回归中20k/200k各三个点查均232 VM步，否定该怀疑；没有重复创建索引或升级schema。

实际红灯：新增SQLite authorizer编译计数测试，旧实现64项INSERT编译64次，期望1次失败。实现把原目标COUNT(*)=1查询和INSERT各prepare一次、批次内重绑参数；1/64/1024项各一SELECT和一INSERT编译。保留原transaction、每项前后check、身份/gap互斥校验、重复ID/缺失目标拒绝及最终取消整批回滚。没有更换身份验证或引入缓存授权。

process_unix相关10/10通过，含20k/200k点查、旧库索引迁移、原生身份记录和真实插入后取消回滚。Store全工作区362通过/9忽略，含原工作区未提交reader_observation_tests两项及对应lib测试挂载，不能冒称精确提交的362项清单。Store全目标Clippy、fmt、OpenSpec严格校验通过。证据在docs/benchmarks/unix_staging_prepare_2026_10_08。

没有Windows端到端加速证据，本路径由Linux扫描使用，不能称Windows200k问题已修复；未勾选生产门禁。当前CI37750244950绑定1d434c9，不包含本批或cedd1a8，等待其终态后再推送，避免取消整轮。
