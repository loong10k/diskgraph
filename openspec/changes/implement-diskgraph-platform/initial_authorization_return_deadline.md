# 初次授权返回的原期限检查

既有有期限 revision/snapshot 入口以原 deadline 打开图连接，但之后同步能力授权不保证触发 SQLite VM handler。隔离合法发布夹具已复现 revision 入口的迟到允许：能力回调在期限内进入、等原 500 ms deadline 到期后返回 Allowed，旧入口返回 Ok。该结果是实际行为 RED，不是原生环境故障。

两有期限入口在原授权成功之后、返回资源许可之前重新检查同一绝对 deadline，迟到允许精确返回 Business(BudgetExceeded)。已观察到的拒权与真实存储错误先返回；不新建期限、不扩大授权、不刷新预算，也不改变没有 deadline 的可信兼容签名。

回归覆盖两个入口的正常允许及迟到允许。合法 Store 发布元数据不使用原生扫描，不能代替平台验收。相关能力测试 3 passed；新源码原生 CI 仍待验收。

限制：同步能力回调和文件系统单调用仍不可硬抢占；初次控制锁在下述补充修复中受原期限约束。不能据本修复宣称整个请求链已有严格墙钟上限。

## 控制锁与控制 SQL 的原期限

补充真实双线程竞争夹具：独立线程持有 Engine 控制锁 300 ms，请求沿原 50 ms deadline 授权。返回时记录 owner 是否仍持锁，最后 join 清理，不仅检查错误值。返回检查版实际 RED：请求等待 owner 释放后才返回。修复后 revision 和 snapshot 两入口均在 owner 仍持锁时返回 Business(BudgetExceeded)。

两入口的归属校验改为获取一次 control_until(deadline) guard，并在同一原期限的两段 with_read_deadline 下读取 server、撤销状态与持久权限交集；宿主能力回调位于两段 SQL guard 之间。SQLite busy、interrupted 与预算耗尽统一映射为业务预算错误。同步授权能力仍不能硬抢占；回调返回后超时不能产生允许结果。无期限可信接口保持兼容。

当前 terminal_capability_tests：4 passed，包含两个入口正常授权、迟到允许、两种入口真实锁竞争及原终检撤权回归。尚未证明所有预算入口、TUI 兼容入口及整个请求链的墙钟上限；这些路径仍需继续审计。当前改动尚待独立审查及同提交平台 CI。

错误优先级：已观察到的真实拒权和能力回调明确 Denied 返回 PermissionDenied。回调耗尽原期限后返回 Allowed 时直接返回 BudgetExceeded，不增加撤权观察宽限，也不承诺读取期限结束后才提交的撤权。不会提交任何迟到授权数据。初始准入与已有末段独立撤权观察窗口的职责不同。

本机最终验证：4 项授权回归通过，其中两个初始入口分别覆盖迟到 Allowed 与迟到明确 Denied；6 项 source_layout 通过；Engine all-targets Clippy -D warnings 与 fmt check 通过。两条独立审查对生产代码返回 APPROVE / CLEAR。真实昂贵 SQL 中断和完整平台验收仍未由本批证明。

## 预算化关系与历史初始授权

随后审计发现 authorize_revision_with_budget 仍调用无期限 owner helper。控制锁竞争夹具增加第三种入口，在修复前精确业务预算断言实际失败；修复后改为传递 reads.deadline() 至同一有期限 owner helper。归属原始字节仍通过 revision_ownership_with_budget 计费，不重建账本或查询期限。

当前授权回归 4 passed（控制锁场景实际覆盖三个入口）；relation_request_tests 为 7 passed / 8 failed，八个失败均在原生扫描夹具初始化 line 110 返回 Business(Unsupported)，不能当成查询行为通过。七个使用已发布元数据的回归覆盖初始授权、末段归属、撤权及历史数据到期语义。原生失败仍需在对应平台 CI 闭环。

初始控制授权 helper 已迁至 initial_revision_authorization.rs，满足生产文件小于 500 行的门禁，lib.rs 仅增加模块声明。拆分后最终授权 4 项、source_layout 6 项、Engine Clippy 与 fmt 均通过；独立两条审查 APPROVE / CLEAR。无期限兼容接口仍保持原路径。

## 可信有期限读取包装的锁等待

with_authorized_revision_reader 的初次与末段控制锁原使用阻塞获取。初次真实锁竞争新增第四入口时实际 RED：不返回精确业务预算错误。两处现使用 control_until(deadline)，没有刷新期限。末段测试在 consumer 准备真实结果 42 后让独立线程持锁 300 ms，原 100 ms 请求返回 BudgetExceeded 且返回时仍持锁，不能交付准备结果。

当前授权回归 5 passed。该包装的 SQL handler、同步回调和撤权优先级尚未全面改造，本批仅关闭控制锁无期限等待；不要据此声称包装已有严格墙钟上限。新增末段测试未单独记录修改前 RED，初次用例已记录实际 RED。
