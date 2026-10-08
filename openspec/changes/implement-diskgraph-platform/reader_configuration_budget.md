# 只读SQLite连接准备的原预算保护

沿用Q-02及原50ms终检观察，不复用旧图库连接，不省略cache/temp_store配置。当前阶段：实现及macOS ARM/Windows定向验收完成，Windows完整Store尝试失败；同源码全平台CI尚未运行，不关闭整体生产门禁。

## 验收行为

1. 打开连接返回后、每条配置SQL开始前及配置返回后均检查同一绝对期限和取消标志；前一步耗尽预算时不得继续下一条SQL。
2. 原SQLite VM执行期限与取消检查在首条配置SQL之前安装，进度间隔仍为1000条VM指令。短SQL须另有明确的阶段边界检查。
3. 每条配置SQL的busy等待使用当前原期限的剩余额度，保持原1秒上限，不复用打开后最初的剩余值。
4. 活跃请求仍执行两条真实配置，保持temp_store=FILE、cache_size=-8192及只读打开标志；运行期取消与原期限优先级不变。

## 实现及实际红绿证据

`SqliteSnapshotStore::open_reader_until`提前安装原progress handler，加入配置前准入，并在每条PRAGMA之前重新设置busy等待。公开签名、错误分类、独立连接、新鲜WAL归属观察、初始原期限均保持。

测试仅在真实连接打开后/真实temp_store配置完成后设置一次性线程局部回调；使用SQLite authorizer记录真实PRAGMA准备，不替换配置结果或模拟SQL完成。

- macOS ARM初轮旧实现：8通过、2失败、2既有诊断忽略。到期后真实配置SQL为2条，取消后真实cache配置为1条，均违反不开始后续SQL要求。
- 增加busy等待检查时，测试误用SQLite不支持的u64 FromSql，出现编译错误；已改为i64读取并检查非负，编译错误不计行为RED，Windows原始编译记录单独保留。
- Windows原生旧实现：8通过、3失败、2既有诊断忽略。前述计数再次为2/1；cache配置busy等待839ms，而此时原期限实际仅剩299ms。
- 修复后两平台定向各11通过、0失败、2既有诊断忽略，含活跃配置正控、运行期取消、准备结束后取消/期限及初始期限优先级。
- macOS ARM Store all-targets：377通过、0失败、11忽略；Clippy all-targets `-D warnings`及受影响源码edition2024格式检查通过。本机包含用户未提交的reader_observation_tests模块及两项测试，保留原修改，不能把377项称作最终commit精确清单。
- Windows默认并发Store all-targets尝试：lib为360通过、3失败、7忽略，61.90秒；后续integration未执行。失败分别为既有process_enqueue_writer_wait_consumes_the_original_remaining_deadline（实际3.187秒返回，已错过writer释放）、既有expiry_during_reader_preparation_refuses_the_prepared_connection（未到达准备末边界），以及新增cache_configuration_busy_wait_uses_remaining_original_deadline（在期望活跃Reader的正控处收到BudgetExceeded）。保留全量反例，不用定向11/0覆盖；未增大测试原期限或串行化测试。Windows Clippy all-targets `-Dwarnings`通过。

最终产品源码SHA-256：`sqlite_snapshot_store.rs`为`f0cb6395e635ede251643b001ef0a664ad5dd68b7db5130611afebc8ab512a53`，测试文件为`213aed96a115081389183776494ceb4bc3d0eac6332f032cdf9a43ab15ec405c`；Windows定向receipt与本机两文件一致。Windows基线Git HEAD为d3fae86b，测试采用结构化增量overlay，并非该HEAD未修改源码。

原始证据在 `docs/benchmarks/reader_configuration_budget_2026_10_09/`：Windows原生归档30427字节，SHA-256为`81d1ef75d1c3053abfba8f73ebaad064fed60bef0d82572f8689e0f4430e5fed`，含独立的夹具编译失败、行为RED、定向GREEN、全量失败、Clippy及原运行脚本和最终两源码；macOS日志归档SHA-256为`eaf1185d616b30e0a2d8a5ee1a8eb5d787871d02af2da3ebc992f6c80981fc29`，另有明确混合工作区边界的receipt。

## 限制和未完成门禁

本项关闭连接配置阶段“预算已耗尽仍开始下一条SQL”的实际缺陷，不证明macOS Intel的89ms/82ms连接打开超限已修复。同步内核/SQLite C调用仍不保证硬抢占；50ms稳定性、原15秒Git完整采样、Windows200k原300秒全负载、有限前端退出和当前同SHA全平台资格均继续开放。没有修改阈值、默认测试并发或既有忽略清单。
