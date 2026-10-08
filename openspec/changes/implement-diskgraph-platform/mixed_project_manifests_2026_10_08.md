# 同目录多清单项目归属回归修复

对应 EV-01 Shared cache、EV-04 Multiple manifests in one directory。此次为原生 Rust 既有采集器修复，不引入新数据 schema、外部命令或文件写能力。

## 复现与行为

旧实现按 parent ID 保存首个生态及首个清单节点。同目录 Cargo.toml/package.json/build.gradle.kts 只有第一项产物得到归属；Cargo/Maven 共享 target 或两种 Gradle 清单共享 build 时，其他归属静默丢失。反转节点遍历顺序还能改变首选项目。

新增两项测试在旧实现实际运行失败（原有9项通过）：不同产物只有1个所有者而非3个，共享产物只有1个所有者而非4个。首次测试编译时使用不存在的 Relation::as_str，已修正为枚举比较后才记录行为红灯，不将编译失败充当问题复现。

实现按 parent 保留全部清单，并独立匹配产物。共享产物的配方、证据和边 ID 加入原始 manifest node ID；单归属保留原 ID，资源实体依旧按 node ID 去重。实体、证据与边稳定排序。采集器和规则版本均升级到3，旧 revision 不改写。Node 产物仍不产生 rebuildable_by，布局依据仍不授予删除许可。

## 验证

- Engine collectors 12/12：包括不同产物、多所有者、同工具清单ID不碰撞、遍历重排、引用闭包，以及真实隔离SQLite暂存→归属绑定发布→revision关系/实体/来源读回。
- Store collector相关回归34/34：发布原子性、迁移、CAS、成员来源与旧版本隔离。
- Engine source_layout 6/6、Engine all-target Clippy -D warnings、workspace fmt、OpenSpec strict及diff检查通过。
- 全量 `cargo fmt --all` 会检查 vendored 上游的既有格式差异，未修改固定 pin 源码；按 CI 明确列出10个工作区package的格式检查通过，原始失败日志保留为fmt.log.gz。
- 原始目标红灯与绿灯日志位于 docs/benchmarks/mixed_project_manifests_2026_10_08。发布测试最初因夹具根不匹配和缺staging被真实校验拒绝，修正夹具后通过；这些夹具错误不作为生产缺陷。

## 边界

未完成应用安装归属EV-06、清单正文/自定义输出解析及未知覆盖的完整资格；本次不关闭这些独立要求。全workspace和目标平台完整回归须由包含本修复的新提交CI补齐；此前d8d3995f平台结果不作为本修复的原生验收。
