# Linux 私有包的 GNU 构建基线

沿用本变更 AI-01/ST-05 与现有 glibc 2.17 包合同，不改变平台基线。当前 b14aa43a CI 38041975682 的 x86 Linux 包任务 114183990290 实际 ABI 报告拒绝：glibc 2.39 exceeds advertised baseline 2.17。私有 package-linux.sh 同样使用普通 cargo build，不能仅修 CI 路径而保留私有打包缺陷。

脚本保持原输出目录参数，只接受本机 Linux GNU x86_64/aarch64；要求已安装 cargo-zigbuild 与可由其定位的 Zig，不自动安装任何工具。显式以 host target 加 .2.17 构建，读取无 ABI 后缀的 target/release 目录，支持 CARGO_TARGET_DIR。原有限 ELF 检查覆盖全部三镜像，检查成功后才创建候选输出目录，安装后再次检查实际包内字节。构建失败、缺工具、错误 OS 或非 GNU host 都不产生候选包。该变更未提交或执行待批准的 CI 工具安装候选。

TDD：新的隔离 Bash 调用回归实际记录旧脚本 cargo build 参数，因缺少 zigbuild/显式 .2.17 target 而失败；已有结构断言也在目标命令缺失处失败。实现后 GNU ABI 相关 19 项通过，覆盖真实脚本参数、构建失败不创建输出及缺工具/非GNU/错误平台拒绝；另有 bash -n 和目标 diff 检查。脚本夹具的 cargo/rustc/uname 为明确测试命令，ELF 为格式夹具，不代表真实编译或目标平台运行。

实际 GNU release 构建、最终镜像 ABI、安装/升级/回滚/负载与同源码完整平台验收仍开放；不将脚本单元测试当作 Linux 生产资格，不提高 2.17 基线，不启用写操作。

后续隔离 Git archive 05fd8aca 的完整本机 workspace 尝试退出101；33目标失败，多项扫描夹具因未提供受信部署材料返回Unsupported、协议driver夹具缺失，不能计为完整通过。该尝试还真实暴露MCP deploy_contract仍断言普通cargo build；同步为Linux平台拒绝、显式zigbuild/.2.17及ABI门禁先于安装的既定新契约。此断言修正不替代实际GNU二进制验收。

归档源tree为b58037ab4afb26f2531ff8cf0b34473a21725944，source.tar SHA256为f551a1752d2d1652b99eb539453218859e6d7efd236a97c38943efe726bede21，原日志在本机隔离目录/tmp/diskgraph-05fd-workspace-P5zOtv/workspace.log。修正后MCP deploy_contract六项通过，目标严格Clippy、rustfmt、diff及OpenSpec strict通过；另外包内worker独立部署七项、workspace CI覆盖两项通过。这些结果均不关闭完整原生workspace门禁。

2026-10-10 用户明确批准仅在 GitHub 临时 runner 安装固定 cargo-zigbuild 0.23.4 与 ziglang 0.16.0。CI 使用 RUNNER_TEMP 内的 venv，不改桌面或全局环境；当前 GNU 三镜像、升级回滚用的旧 CLI 与 worker receipt 构建均显式使用 `.2.17`，保留实际 ELF 门禁。官方 PyPI 两个版本均提供 x86_64/aarch64 Linux wheel；尚不代表编译成功。新增 CI 合同测试对已提交旧工作流实际失败（缺少隔离编译器步骤），候选四项通过；19 项 ABI 回归通过。源 671fd4e1 的实际 x86_64 ELF 仍被拒绝为 glibc 2.39，来自 run38045974868 artifact11666683759，不将该源码包标为合格。新工作流实际构建及完整包验收待执行。

源 6c1c26d1 的 run38046270976 两种 GNU runner 已实际安装并运行固定工具，但 release 链接均失败：生产代码强引用 glibc 2.17 不提供的 `statx` 与 `memfd_create`。这是实际二进制构建 RED，不是工具缺失。对应调用改为 `libc::syscall(SYS_statx, …)` 与 `libc::syscall(SYS_memfd_create, …)`，沿用原参数、返回/errno、唯一挂载能力检查及执行镜像密封；不增加较弱回退，不修改 vendored 上游。Rust 标准库自己的弱 statx 探测不替换、不导出全局 statx 符号。修复后的 Linux 链接、原生 syscall/密封行为、实际 ELF 及包验收仍待 CI；macOS 格式检查不代表 Linux 编译通过。

后续实际验收：ba2aa6ec 的 run38046631229，x86_64 job114197521380 与 ARM64 job114197521415 两个 GNU 原生包任务均成功。实际包验收分别执行 stdio18、HTTP14、升级回滚7、20k负载6、200k负载6；两个规模均精确核对完整路径、20001/200001节点、4客户端查询及超限不发布。下载归档校验 SHA256 后，对最终包内三镜像重新解析 ELF，最高需求仍为2.17；包内worker长度/摘要与独立Cargo快照和原checkout SHA一致。注意只构建worker的后续产物摘要与首次三镜像构建可不同，最终证据采用实际包字节及对应独立回执，不混用首次ABI报告中的worker摘要。

持久证据：`docs/benchmarks/linux_gnu_package_ba2_2026_10_10/receipt.json`及同目录原始负载/HTTP、最终ABI、包汇总和独立worker回执。200k单轮x86扫描22.180秒、查询p50/p95 7.008/9.218ms；ARM64扫描20.579秒、p50/p95 13.996/19.169ms。不据跨runner单轮数据声称性能提升，也不称RSS/peak WAL已测量或glibc2.17旧userland已实际运行。整个同源码workflow仍未终态，父生产任务保持开放。
