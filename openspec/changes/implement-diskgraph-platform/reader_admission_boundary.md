# Q-02：独立只读连接准入边界

连接准备前及配置后检查同一原绝对期限和取消标志。期限已耗尽返回 BudgetExceeded，取消保持 SQLite interrupted 分类；同时成立时期限优先。配置过程中的真实 SQLite 失败保持原错误，连接失败后由 RAII 释放。生产不包含测试注入钩子，不更新期限，不复用消费者连接。

已验证：初始三项目标回归因缺少入口检查失败；修复后七项目标回归通过，包含准备过程中取消/到期、同时失效优先级及运行期 SQL 中断。macOS arm64 store 回归 305 passed、0 failed、1 ignored；workspace check、store Clippy 与 fmt 通过；独立代码审查 APPROVE，架构审查 CLEAR。

日志和文件指纹：`docs/benchmarks/reader_admission_boundary/receipt.json`。跨平台候选 CI 尚待验收。同步打开及配置仍为协作式检查，不承诺硬墙钟抢占；检查后的取消竞态仍需执行/返回门禁。该修复不代表可信 supervisor 启动、前台有限退出或完整生产验收已完成。

## English acceptance note

Independent reader setup checks the original deadline and cancellation before accessing the database path and again before returning the configured connection. Expiry maps to BudgetExceeded; cancellation preserves SQLite interrupted classification. Expiry wins when both conditions hold. Existing setup errors are not overwritten. The runtime progress handler retains the same cancellation state; test hooks are absent from production builds.

Local acceptance: three missing-admission regressions failed first; seven targeted tests now pass, including expiry/cancellation during setup and runtime cancellation. The store regression suite passes 305 tests with one ignored test. Workspace check, store Clippy, formatting and two independent review lanes pass. Native candidate CI and the separate supervisor/foreground recovery gates remain incomplete; this is not a production-ready verdict.
