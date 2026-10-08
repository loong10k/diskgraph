# 发布校验连续ID的有界辅助索引

整体生产门禁仍开放，不勾选父项或RSS/Windows SLO。

## 实现边界

原适配器转换以nodes.len()+1生成并按顺序追加ID，Store仍不信任这一来源假设。发布验证先逐项检查ID等于first+index且加法无溢出，仅全部成立才使用范围索引；非连续、逆序、重复及旧图保留原HashMap路径。查找使用checked_sub及usize范围核验，支持非1起点、0和u64边界；不重排输入、不改节点编号、不新增持久数据。不改变重复ID/缺父/根/循环/别名/覆盖/证据的拒绝顺序，不移除staging、原始定位、授权、fence、回执或提交检查。图索引不代表文件身份或权限。

## 验证和成本

新增编号/逆序/极值/错误优先级合同在原实现通过，再验证优化后保持通过；原连续/逆序2万层父链回归保持。公开checked发布准备在第二个原检查点返回BudgetExceeded，真实事务回滚、无快照；独占子进程测量同一20k图的连续与逆序Rust requested allocation。原实现两者872144字节，新分配合同真实失败；优化后连续315080字节、逆序872144字节，合同通过。不将Rust requested累计值当峰值、SQLite C分配或RSS。

同机release合成200k元数据、4预热/20采样的一次前后实验：p50 18.919→11.849ms，p95 20.778→12.738ms。仅发布准备与拒绝事务；没有真实文件扫描、成功发布、进程RSS或交替轮次证据，不能据此关闭整体性能门禁。

publication回归9通过/1默认忽略；手工release测量分别显式1/1；Store全库344通过/6忽略（含用户未提交的2个reader observation测试）；Store all-targets Clippy、fmt、diff检查、OpenSpec strict通过。最终提交原生平台CI仍待运行；本轮保留原先活跃的两个Windows完整工作区测试，不因观察到时未结束而重启。

## 新获得的原生失败证据

616958e的macOS Intel完整Engine716通过/1失败/13忽略，唯一失败仍是旧sleep20ms的terminal contention，返回时持锁未释放、终检54.418ms，属于先前测试同步修正所覆盖的触发方式；不能将其终态改为通过。完整日志无损保留。原生包macOS Intel已通过，单独包成功不覆盖该Rust失败。

原始日志及每份解压摘要见docs/benchmarks/publication_dense_ids_2026_10_08/summary.json。本轮没有修改vendored上游。

追加当前源码验证：source_layout1/1、受影响Engine lock-gap3/3通过。旧616958e Windows stable的完整workspace Test步骤已真实成功，job仍在后续测量；不将步骤成功写为整个job或本次新源码通过。保存原API观察，不提前打断剩余Windows MSRV。
