# Linux 私有包的 GNU 构建基线

沿用本变更 AI-01/ST-05 与现有 glibc 2.17 包合同，不改变平台基线。当前 b14aa43a CI 38041975682 的 x86 Linux 包任务 114183990290 实际 ABI 报告拒绝：glibc 2.39 exceeds advertised baseline 2.17。私有 package-linux.sh 同样使用普通 cargo build，不能仅修 CI 路径而保留私有打包缺陷。

脚本保持原输出目录参数，只接受本机 Linux GNU x86_64/aarch64；要求已安装 cargo-zigbuild 与可由其定位的 Zig，不自动安装任何工具。显式以 host target 加 .2.17 构建，读取无 ABI 后缀的 target/release 目录，支持 CARGO_TARGET_DIR。原有限 ELF 检查覆盖全部三镜像，检查成功后才创建候选输出目录，安装后再次检查实际包内字节。构建失败、缺工具、错误 OS 或非 GNU host 都不产生候选包。该变更未提交或执行待批准的 CI 工具安装候选。

TDD：新的隔离 Bash 调用回归实际记录旧脚本 cargo build 参数，因缺少 zigbuild/显式 .2.17 target 而失败；已有结构断言也在目标命令缺失处失败。实现后 GNU ABI 相关 19 项通过，覆盖真实脚本参数、构建失败不创建输出及缺工具/非GNU/错误平台拒绝；另有 bash -n 和目标 diff 检查。脚本夹具的 cargo/rustc/uname 为明确测试命令，ELF 为格式夹具，不代表真实编译或目标平台运行。

实际 GNU release 构建、最终镜像 ABI、安装/升级/回滚/负载与同源码完整平台验收仍开放；不将脚本单元测试当作 Linux 生产资格，不提高 2.17 基线，不启用写操作。
