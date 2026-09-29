# P9 移动平台验收边界（任务 10.1–10.9）

日期：2026-09-29 · 主机：macOS（darwin 27.0.0，arm64）· 结论：**P9 九项全部不勾选**，逐项如实留档如下。这些场景被机器可查清单 `diskgraph_testkit::real_os_requirements()`（`MobileProviderLifecycle` 等）覆盖，验收不属于"可以模拟"的类别（RE-02）。

## 本机具备与缺失

| 项 | 状态 |
| --- | --- |
| Rust Android 交叉 target（aarch64/armv7/i686/x86_64-linux-android） | ✅ 已安装 |
| Android cmdline-tools / build-tools / platform-tools / 模拟器组件 | ✅ 存在（homebrew） |
| Android NDK | ❌ 未安装——`cargo check --target aarch64-linux-android -p diskgraph-ffi` 停在 `libsqlite3-sys` build-script（需 NDK clang 编译 bundled SQLite） |
| 完整 Xcode（xcodebuild） | ❌ 仅 Command Line Tools——XCFramework 打包与 iOS 模拟器切片不可行 |
| Android/iOS 真机 | ❌ 无 |

## 逐项留档

- **10.1 Android NDK 构建 + AAR + Kotlin 绑定测试**：NDK 缺失使交叉编译（含 SQLite）无法完成；AAR 打包另需 Gradle（本机无）。绑定层已独立验证过：Kotlin 绑定源经 kotlinc + JNA 真实编译运行（见 `docs/acceptance/ffi-bindings.md`），该证据覆盖"绑定可用"，不覆盖"Android ABI/原生库加载"。
- **10.2–10.4 SAF DocumentUri provider、宿主授权生命周期、动作能力**：全部需要 Android 设备或已配置 NDK/Gradle 的构建环境；本仓库无 Android 宿主工程。拒绝退化语义（不支持动作不得退化为永久删除）已在 ops 层成立（trash 无回收能力时报 unsupported，OP-06 测试），但 SAF 侧语义无从验证。
- **10.5 Android 真机验收**：无设备。
- **10.6–10.8 iOS 切片、安全作用域/书签、文档 provider 动作**：无 Xcode、无模拟器 runtime、无设备。
- **10.9 iOS 真机验收**：无设备；且规格本身要求"不以模拟器编译代替真机验收"。

## 恢复路径

在装齐 NDK + Gradle 的主机上：`cargo build --target aarch64-linux-android -p diskgraph-ffi --release` → uniffi 生成 Kotlin 绑定 → Gradle 打 AAR → 模拟器冒烟。iOS 同理需完整 Xcode。此前置条件与 9.9 的 XCFramework 打包共享。
