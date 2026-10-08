# Staging 搜索写入语句复用

## 同源码 Windows 原生诊断

CI 37744394886 / 5dc825d 的 Windows完整200k诊断（独立900秒诊断，不是300秒正式验收）完整6/6成功：夹具190.619s，索引179.206s，查询p95 43.650ms，数据库及WAL合计553598976字节。索引阶段：worker完成16.619s、转换21.507s、staging完成127.775s、发布完成179.022s。**约106s staging区间包含逐节点原生观测和SQL写入，不能全部归因为SQLite；本次没有证明其中的具体占比。** 含清理总耗时578.102s，正式验收仍超时失败。

归档原始产物：docs/benchmarks/windows_load_phases_5dc825d_2026_10_08。产物包含二进制/脚本摘要与commit，production_acceptance=false保持不变。20k夹具线程诊断不同轮次排序不稳定，不能据此改动正式并行度或排除创建/清理时间。

## 已证实的重复工作及修复

scan_staging_store 中主节点 INSERT已复用prepared statement，但搜索INSERT在每个节点通过execute重新prepare。SQLite真实authorizer计数的旧实现64行编译64次，目标测试失败；候选将搜索statement移到同一批事务内的循环外，逐行只绑定值。

1/64/8192行均只编译一次，所有Unicode小写搜索值和顺序完整核对。每节点取消检查、原事务commit前检查和rollback语义不变。新版回归1通过、原job_publication_check_tests 5通过、Store all-targets Clippy通过。日志见docs/benchmarks/staging_search_prepare_2026_10_08。

本次证明编译工作量从每节点一次变为每批一次，不声称179秒扫描已变快或Windows已达到300秒门槛。所有新修复还需同SHA目标平台验收；整体生产就绪任务不勾选。
