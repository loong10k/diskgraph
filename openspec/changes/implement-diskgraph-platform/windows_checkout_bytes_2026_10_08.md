# Windows checkout 源码字节一致性

Windows 台式机固定提交 1ac266a0 的工作区为干净状态，但 core.autocrlf=true。实际读取 scripts/qualify-linux-atomic-launcher.py 和 integration_candidates/native_child/linux_atomic_abi.rs，分别含 443 和 29 个 CRLF；仅移除 CR 即与 HEAD blob 完全一致。原生 Rust 编译通过不等于受审源码摘要一致，不能把此类资格校验失败计为通过。

验收场景：在隔离 Git 仓库启用 core.autocrlf=true，使用本项目属性文件实际 add 与 checkout-index 后，受摘要保护的 Python、Rust、diff、JSON 源字节仍与 LF 输入一致。二进制字节和 vendored disktree 的原始 CRLF 必须原样保留，不修改上游 pin、源码或摘要。

修复采用 Git 文本自动检测与统一 LF checkout，继续以更具体的 vendored -text 规则禁止改写上游字节。不修改用户全局 Git 配置，不覆盖台式机进行中验证的源文件。本轮源码字节回归与 Windows 产品运行验收分别记录；已有 checkout 的历史 CRLF 不因新增属性文件自动变为已验证状态。

docs/benchmarks 的原始验收材料同样采用 -text，避免换行规则改写已记录摘要的 stdout、日志或历史捕获。该保护单独加入同一实际 checkout 回归。

实际验证：本机原属性文件为 1 失败 / 1 通过，失败原因即 LF 被 checkout 为 CRLF；增加属性后 2/2 通过。候选属性与相同测试传入 Windows 独占临时目录，已有 Python 3.13.12 实际执行 2/2 通过；未修改该机受测 checkout。Windows 输出摘要为 4fdc3c88183a53ae475e970b6418f046f61c9c61a2a509ebbf442a0aa949aaa2，回执见 docs/benchmarks/windows_desktop_1ac266a0_2026_10_08/native_summary.json。
