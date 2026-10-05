# 2026-10-06 桌面只读原生验收记录

规格事实源仍为本变更的 physical-scan-process.md、specs 与 tasks。该记录不关闭任何父任务，也不声明发布版本或全平台验收完成。

| 源码 / 实际运行 | 观察结果 | 结论边界 |
| --- | --- | --- |
| d957e66 / Windows CI37384689462 | 编译通过，四项跨线程 pending I/O 通过，三项清理恢复 RED | 真实原 owner 提前丢失与槽位释放问题已复现 |
| 436ebea / Windows CI37385193355 | 原七项全部实际通过 | 清理失败保留原句柄与真实重试已验证；出生与清理期限仍开放 |
| 7f2b5e8 / Windows CI37385724802 | 原七项继续通过，新增三项出生后错误/panic owner RED | 原错误/panic 保留，catch 外 owner 槽为空；待外槽出生实现 |
| 8878912 / macOS CI37381931634 | 临时卷 mkdir 非法 UTF-8 返回 EILSEQ，未进入扫描 | 不能作为 helper 扫描失败；必须修正实际文件名夹具 |
| 66c445a / macOS CI37385115407 Intel 产物 | 协议、root 安装、helper 正常/取消/panic、Engine 发布具备实际 PASS 标记；响应预算案未出生 | 原 32 字节夹具不能通过 Hello/预算终态准入，不是出生后溢出证据；记录产物身份，不替代尚未终态的任务状态 |
| 当前 macOS budget focused test | 显式候选 feature 下精确 1/0/0；512 字节真实准入，实际树编码 992，Hello+预算终态观察 230 | 仅本机原 scanner/codec；实际 helper 与 Engine 拒绝发布须由新 CI37386707636 验证 |

原始 stdout/stderr、receipt、candidate 源码摘要与实际测试程序摘要位于 docs/benchmarks/windows_cleanup_native_red_2026_10_06、windows_cleanup_native_green_2026_10_06、windows_birth_owner_native_red_2026_10_06、macos_installed_invalid_filename_failure_2026_10_06 和 macos_engine_budget_admission_failure_2026_10_06。

仍须完成 Windows 出生前外部 owner、探针的持久恢复域、有限协作清理、受信镜像与完整 helper/Engine/CLI/MCP 正向链路；Linux 旧 Conflict 首因、三平台内容/provider、当前全 workspace 与同 SHA 性能/原生门禁仍开放。资格快照不能代替当前未提交集成源码的完整验收。

CI37386707636 的实际 job112021625493 labels 为 macos-15-intel，回执 runtime architecture=x86_64，源码 c0ea6ca。两个协议/预算条件、一个 root 安装更新恢复、六个普通 helper/Engine 案均精确 PASS（9/9）；原错误/panic 文本属于通过案的预期取证，不单独算失败。原回执及九案日志保存于 docs/benchmarks/macos_installed_engine_native_green_2026_10_06。ARM job112021625944 待完成；此结果不关闭 CLI/MCP 正向、并发更新/发布或完整全平台门禁。

## Windows managed probe RED preparation

The fixed candidate now includes three exact production-probe regression cases: cancellation with native wait failure, cancellation with Job query failure, and the original String panic with native wait failure. They bind a real fixed-capacity ProbeHost but deliberately retain the original probe execution algorithm. Actual birth is independently witnessed; acceptance requires the original error or panic, retained owner/capacity, rejection of another reservation, and native wait/Job-zero observations before releasing the slot. No missing API, zero-test result, or compile failure counts as behavioral RED.

The prior 17 native cases remain unchanged. Local qualifier checks passed 7/7 and archive safeguards 8/8; native Windows execution remains pending. The frozen 466-file archive SHA-256 is `d35a44e6f5eef656365ac049b0bc233eccbe1ca335cf82121f18c3afcbea13a0`. These tests do not qualify the complete product.

GREEN must also transfer the corresponding Git private-directory owner with the failed child into the same bounded session recovery responsibility. Directory completion or Drop must not remove a directory still used by a retained child, nor retain unrelated sessions based on a global occupied count. Parent task 15.13 and full-platform readiness remain open.

## Latest native comparison and ARM64 completion

macOS run 37386707636 is now terminal success on both architectures. ARM64 job 112021625944 actually passed all nine cases, each with an exact 1 passed / 0 failed / 0 ignored result; its ordinary UID, source SHA, helper and fixture hashes are retained in `docs/benchmarks/macos_installed_engine_native_green_2026_10_06/arm64/`. This does not close positive CLI/MCP, update/publication concurrency or complete integration.

Windows same-runner run 37388084630 completed: current frozen source passed 14/17, original frozen source passed 11/17. Both failed the same three unchanged normal-exit exact-count assertions (2 versus 1); only the original additionally failed the three birth-owner cases. The original receipts and all logs are retained under `docs/benchmarks/windows_same_runner_baseline_comparison_2026_10_06/`. Before changing any assertion, observe the actual original Job member IDs and native process identities. Parent tasks remain open. Managed-probe RED run 37389541495 is pending.

## Windows member identity diagnostics

The next frozen candidate adds only cfg(test) read-only Job member diagnostics to the normal-exit fixture: at most four nonzero observations per fixture and 64 PIDs per observation, without retries. It logs the original held leader PID, actual Job PID list, queried-handle Job membership, native creation/exit FILETIME and image path, with original error codes and elapsed observation time. Query handles close immediately; no child birth, termination, original assertion or production normal-exit algorithm changes. Enumeration and process-handle observations are explicitly non-atomic and do not prove normal exit. The 467-source archive SHA-256 is `316c1e7377a5767d7c9af791dda372b06ff0f1f1ec661b2fa327f6b05a2d7c79`; qualifier 7/7 and archive checks 8/8 passed locally. Native execution remains pending.
