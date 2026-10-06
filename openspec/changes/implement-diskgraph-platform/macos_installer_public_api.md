# macOS 可信安装程序入口验收

属于既有 PF-06 / 15.6 安装与升级合同，不修改只读 CLI/MCP 的权限，不完成生产父项。

外部 Rust 安装程序必须能够调用 MacosInstallationPublisher::prepare、publish 和 recover。prepare 只建立固定保护布局；publish 只接受原持有镜像、独立宿主预期、可信签名私钥、递增 epoch 与非零安装 ID；recover 只能完成持久 floor 已承诺的一代。不得增加请求指定路径、自动提权或普通 CLI/MCP 安装工具。

公开入口必须先传播原检查点错误及过期期限，再执行身份或文件系统操作。普通真实 UID 或有效 UID 不为 root 时，三个入口均返回 Unsupported，不写固定安装目录。测试从 crate 外部调用真实公开 API，验证普通 UID 拒绝、原错误传播和过期拒绝；不能以内部模块测试代替公开可达性。

实际 root 安装、签名镜像启动、升级恢复、同 SHA 默认 Engine 链路仍需原生隔离 CI 验收；本机不执行安装，不能凭公开 API 或负向测试开启默认扫描。

## 当前实现与证据

公开 prepare/publish/recover 已接入现有真实 bootstrap/发行/恢复实现，没有重复实现或 lint 屏蔽。外部调用先确认导出缺失导致编译失败；这只是 API 缺失 RED，不称为原生行为 RED。补齐后本机普通 UID 三个负向用例 3/0/0；发行回归 11/0、bootstrap 3/0、源码门禁 6/0。原始日志和源码摘要见 docs/benchmarks/macos_installer_public_api_2026_10_06。严格全 feature Clippy 仍被既有未使用兼容入口和候选夹具常量断言阻塞；全平台安装及默认扫描未完成，父项保持未勾选。
