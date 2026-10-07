# Windows 原生 Clippy 构建门禁修复

事实源仍为 implement-diskgraph-platform。实际失败证据：CI run 37581237942，Windows stable job 112661149766，源码 1c9fe464c586d2c1db203ce092d13c3d0b06a830；Clippy 报告17处错误。包含7处 doc comment 空行、6处整除谓词、OVERLAPPED 默认后赋值（生产和测试重复诊断）、目录记录测试默认后赋值和runner嵌套条件。

修复只采用等价表达：保留原 OVERLAPPED 堆地址/UnsafeCell与事件寿命；长度、偏移、完整ID检查不放宽；drain 仍只调用一次并传播原错误。Rust最低版本1.97支持整除谓词和let-chain，不加入allow抑制。

本机macOS Engine all-target Clippy -D warnings通过，代码APPROVE、架构CLEAR；本机未安装Windows target，不能验证条件编译代码。新提交必须由同SHA的Windows stable/1.97原生CI确认。目录版本竞态、监督产品启动链及整体生产门禁仍未完成，父任务不勾选。

## 原生编译反馈

b3ed055 的 Windows stable与1.97 job112680860002/112680860490均实际失败：目录decoder测试后续仍需将原 FileId 清零，但等价初始化重写遗漏原mut（E0594）。恢复可变绑定，不移除零ID拒绝用例。此前本机Clippy不覆盖Windows条件编译；新原生结果不能视为17条门禁已全部关闭，须后续同SHA重新验证。

## 11dd958 原生 stable 后续门禁

Windows stable job 112687983792 已通过完整 Test 步骤，Clippy 在 git_source_windows.rs:60 报 unused_enumerate_index：索引只在 cfg(test) 阶段诊断中使用，生产却仍 enumerate。修复使 enumerate 仅在测试配置生效，生产按原顺序遍历组件；两种配置都保留逐组件 probe.check、child 身份检查和 route capture。不添加 lint allow，不放宽状态比较。Windows 最终 Clippy 需后续同提交原生 CI 验证，不能以 macOS check 代替。
