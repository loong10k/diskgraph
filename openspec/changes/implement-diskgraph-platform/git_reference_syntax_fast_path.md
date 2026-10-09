# Git 引用语法快速校验

当前 Windows 同提交完整 Engine 回归 `92aaa82b` 为 711/6/7，六项均在原 Git 采样期限内失败；不能把本次优化提前称为根因修复。

验收：普通完整引用名按 Git `check-ref-format` 默认规则执行纯字节语法校验，合法名称不启动仅验证语法的子进程。不得以名称校验替代引用存在性、当前 HEAD/分支、目标 commit OID 或终检，不缓存这些运行时结果。快速校验未通过仍保留原 Git 验证和错误传播，不能把非法名称当作缺引用。名称来自原有累计输出预算，不放宽整次默认 15 秒及资源/清理门禁。

按 https://git-scm.com/docs/git-check-ref-format 的完整默认语法验证全部禁止项，覆盖 Unicode、单字节控制字符、`.lock`、路径组件、`@{`、带 dash 的合法完整引用；与实际 Git 做差分。真实 Git/发布回归和默认并发完整 Engine 须另行执行，保持原失败记录。同 SHA CI 与生产总门禁仍未关闭。

Windows 原生边界回归先 RED：2 通过、3 失败，失败证明合法名称仍调用原生语法命令。实现后 7/7 通过，其中 33 个合法/非法名称与实际安装 Git 的退出状态逐项一致；全部 ASCII 控制字符另行覆盖。未改变 HEAD 两次原生读取、unborn 两次存在性检查、非法新分支及原生错误传播。Engine 全部目标 Clippy 通过。证据：`docs/benchmarks/windows_git_reference_syntax_92aaa82_2026_10_09.json`；原完整失败：`windows_engine_full_92aaa82_2026_10_09.json`。

候选源码的完整默认并发 Engine 回归已结束：715 通过、4 失败、7 忽略，库测试 238.15 秒，Cargo 256.594 秒。三个 Git 正向采样仍超时，一个历史数据到期后的终检返回 BudgetExceeded；库失败后未进入后续集成测试。前后失败集合不同，不能用 6→4 宣称性能收益或根因修复。源码前后摘要相同，实际扫描 worker 与当前源码 driver 均冻结并校验摘要。完整证据：`docs/benchmarks/windows_engine_reference_syntax_full_92aaa82_2026_10_09.json`。本轮只完成引用语法的行为优化；全量回归及生产门禁保持未通过。
