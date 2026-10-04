# P9 移动平台验收边界（任务 10.1–10.9）

> 2026-10-04 源码与环境记录复核：P9 不仅缺设备证据，还缺生产 SAF/iOS provider、宿主授权生命周期和移动包工程。环境以[只读盘点](../benchmarks/full_platform_device_inventory_2026_10_04.json)的记录时间为准；不得用旧 target 表或桌面 Kotlin/JNA、Swift/GRDB 测试代替 Android/iOS 验收。

日期：2026-09-29 · 主机：macOS（darwin 27.0.0，arm64）· 结论：**P9 九项全部不勾选**，逐项如实留档如下。这些场景被机器可查清单 `diskgraph_testkit::real_os_requirements()`（`MobileProviderLifecycle` 等）覆盖，验收不属于"可以模拟"的类别（RE-02）。

## 本机具备与缺失（2026-10-04 盘点，旧执行事实另注明）

| 项 | 状态 |
| --- | --- |
| Rust Android 交叉 target（aarch64/armv7/i686/x86_64-linux-android） | 未安装；Oct4 记录只有 `aarch64-apple-darwin`，Sept29 的已安装状态已过期 |
| Android 工具与模拟器组件 | Oct4 仅证实 adb 可执行、连接设备为零；SDK/build-tools/模拟器组件完整性未重新验收 |
| Android NDK / Gradle | Oct2 记录缺失；Oct4 未新增安装或构建证据。旧交叉编译在 bundled SQLite 的 NDK clang 前置失败，不能当 ABI 验收 |
| 完整 Xcode（xcodebuild） | Oct4 选中 Command Line Tools，xcrun xcodebuild 不可用，标准应用目录未找到 Xcode；不能推断所有自定义位置或远程环境 |
| Android/iOS 真机 | Oct4 adb 连接设备为零；没有本任务 iOS 真机验收记录 |

## 逐项留档

- **10.1 Android NDK 构建 + AAR + Kotlin 绑定测试**：NDK 缺失使交叉编译（含 SQLite）无法完成；AAR 打包另需 Gradle（本机无）。绑定层已独立验证过：Kotlin 绑定源经 kotlinc + JNA 真实编译运行（见 `docs/acceptance/ffi-bindings.md`），该证据覆盖"绑定可用"，不覆盖"Android ABI/原生库加载"。
- **10.2–10.4 SAF DocumentUri provider、宿主授权生命周期、动作能力**：生产 provider 枚举／分页／批次导入、持久 URI grant、撤权／重启与后台协调均未实现，本仓库无 Android 宿主工程。Core 的 URI／未知大小模型和测试 provider 不构成可用链路；FFI `document_uri_scan=false`，不能仅放开标志或让 URI 进入原生路径扫描。实际构建和设备验收仍是另一步。拒绝退化语义（不支持动作不得退化为永久删除）已在 ops 层成立（trash 无回收能力时报 unsupported，OP-06 测试），但 SAF 侧语义无从验证。
- **10.5 Android 真机验收**：无设备。
- **10.6–10.8 iOS 切片、安全作用域/书签、文档 provider 动作**：缺 iOS 宿主工程、security scope／bookmark 生命周期、文件协调和文档 provider 生产实现，也缺已验收的切片与 XCFramework；当前环境盘点未提供可运行 Xcode/设备证据。
- **10.9 iOS 真机验收**：无设备；且规格本身要求"不以模拟器编译代替真机验收"。

## 实施与验收路径

先实现 Android 只读 SAF 纵切：宿主持有授权生命周期 → URI 元数据分页批次 → 共享 durable job／fence／staging／revision → unknown／partial 查询 → 撤权／重启／取消验收；不能用已有原生路径扫描替代。iOS 需要对应文档 provider 与安全作用域链路。

构建前置具备后，在装齐 NDK + Gradle 的主机上：`cargo build --target aarch64-linux-android -p diskgraph-ffi --release` → uniffi 生成 Kotlin 绑定 → Gradle 打 AAR → 模拟器冒烟。iOS 同理需完整 Xcode。此前置条件与 9.9 的 XCFramework 打包共享。
