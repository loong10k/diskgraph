# Windows 无人值守 DLL 加载验收修复

2026-10-10：沿用 implement-diskgraph-platform 的恢复后真实加载验收。原期限 20 秒、签名策略及加载正负断言保持不变。

隔离源码由已验证的 5fd11d6c 归档投影到 016bcd99 的两个 Git 引用源码变更，再叠加本次测试修改；不是完整 016bcd99 Git 树，也不是生产安装验收。原 Windows checkout 和旧验证源码均未修改。

红灯：原测试独立运行仍超时。新增只读诊断显示控制 Closed、leader 未退出、双 EOF 未到、Job 活动数 2、加载结果未生成。原 Job 成员为实际测试 executable 与 conhost.exe；阶段记录停在 unsigned-load。不能把 conhost.exe 本身称为错误弹窗或安全漏洞。

修复：真实加载夹具线程调用 SetThreadErrorMode(SEM_FAILCRITICALERRORS)，让系统加载错误返回线程，避免默认 critical-error 对话框阻塞无人值守测试。只影响测试线程的错误呈现，不更改出生时签名政策、不模拟加载结果、不延长期限。依据 Microsoft SetThreadErrorMode 文档：https://learn.microsoft.com/en-us/windows/win32/api/errhandlingapi/nf-errhandlingapi-setthreaderrormode 。

绿灯：Windows stable 原生实际编译通过；同一个测试 binary SHA256 562ac8afb80eff59085945268fe8095e5f114a60beaefe0800869cbf44729b0c：WorkerControl 测试 1/0，12.50 秒，unsigned_error=577、unsigned_loaded=false、system32_loaded=true；Null 正向控制 1/0，0.91 秒，同一有效 unsigned DLL 能实际加载。包装器退出 0，原 Job 退休检查完成后返回。原始工具回执：docs/benchmarks/windows_dll_loader_016b_2026_10_10/native_observation.json。

静态检查：同一隔离源码 Windows stable 的 engine/cli/mcp all-target Clippy --locked -- -D warnings 通过，36.44 秒，原 Job 退休已确认；当前源码 rustfmt、git diff --check 和 OpenSpec strict 验证通过。

本证据只关闭已复现的 DLL 夹具弹窗阻塞，不关闭完整 Windows workspace、Linux glibc 2.17 打包、产品监督退出链或全平台生产门禁。须在提交后的同 SHA CI 中验证；本变更不启用危险写能力。
