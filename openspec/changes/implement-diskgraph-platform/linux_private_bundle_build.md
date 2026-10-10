# Linux 私有包的 GNU 构建基线

沿用本变更 AI-01/ST-05 与现有 glibc 2.17 包合同，不改变平台基线。当前 b14aa43a CI 38041975682 的 x86 Linux 包任务 114183990290 实际 ABI 报告拒绝：glibc 2.39 exceeds advertised baseline 2.17。私有 package-linux.sh 同样使用普通 cargo build，不能仅修 CI 路径而保留私有打包缺陷。

脚本保持原输出目录参数，只接受本机 Linux GNU x86_64/aarch64；要求已安装 cargo-zigbuild 与可由其定位的 Zig，不自动安装任何工具。显式以 host target 加 .2.17 构建，读取无 ABI 后缀的 target/release 目录，支持 CARGO_TARGET_DIR。原有限 ELF 检查覆盖全部三镜像，检查成功后才创建候选输出目录，安装后再次检查实际包内字节。构建失败、缺工具、错误 OS 或非 GNU host 都不产生候选包。该变更未提交或执行待批准的 CI 工具安装候选。

TDD：新的隔离 Bash 调用回归实际记录旧脚本 cargo build 参数，因缺少 zigbuild/显式 .2.17 target 而失败；已有结构断言也在目标命令缺失处失败。实现后 GNU ABI 相关 19 项通过，覆盖真实脚本参数、构建失败不创建输出及缺工具/非GNU/错误平台拒绝；另有 bash -n 和目标 diff 检查。脚本夹具的 cargo/rustc/uname 为明确测试命令，ELF 为格式夹具，不代表真实编译或目标平台运行。

实际 GNU release 构建、最终镜像 ABI、安装/升级/回滚/负载与同源码完整平台验收仍开放；不将脚本单元测试当作 Linux 生产资格，不提高 2.17 基线，不启用写操作。

后续隔离 Git archive 05fd8aca 的完整本机 workspace 尝试退出101；33目标失败，多项扫描夹具因未提供受信部署材料返回Unsupported、协议driver夹具缺失，不能计为完整通过。该尝试还真实暴露MCP deploy_contract仍断言普通cargo build；同步为Linux平台拒绝、显式zigbuild/.2.17及ABI门禁先于安装的既定新契约。此断言修正不替代实际GNU二进制验收。

归档源tree为b58037ab4afb26f2531ff8cf0b34473a21725944，source.tar SHA256为f551a1752d2d1652b99eb539453218859e6d7efd236a97c38943efe726bede21，原日志在本机隔离目录/tmp/diskgraph-05fd-workspace-P5zOtv/workspace.log。修正后MCP deploy_contract六项通过，目标严格Clippy、rustfmt、diff及OpenSpec strict通过；另外包内worker独立部署七项、workspace CI覆盖两项通过。这些结果均不关闭完整原生workspace门禁。
