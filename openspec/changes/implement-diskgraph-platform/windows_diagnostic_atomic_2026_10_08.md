# Windows诊断计数器工具链兼容修复

对应既有RE-04/07同SHA原生验收门禁，不改变验收标准或任务完成状态。

CI `37764626953`（`72e6be2`）的Windows stable Rust、Windows release包与Windows Kotlin三个job均在编译engine时因`AtomicU64::fetch_update`弃用且`-D warnings`而失败，未进入后续业务验收。macOS Intel Swift在rustup下载stable工具链时DNS解析失败，是独立环境失败。原日志与解压后摘要保存于`docs/benchmarks/windows_diagnostic_atomic_2026_10_08/`。

将原饱和增量改为稳定的`compare_exchange_weak`循环。保持Relaxed顺序、饱和语义、原错误与panic传播；竞争失败使用最新观测值重新计算。没有抑制warning、放宽门禁、删除根链检查或修改vendored源码。

本机rustc 1.98.1并未复现远程工具链的弃用红灯，因此红灯证据为上述原生CI。直接以`rustc --edition=2024 -D warnings --test`编译该纯标准库模块，4/4通过，覆盖原结果、原panic、饱和与8线程80000次增量无丢失。此检查实际编译Windows使用的计数器源码，但不代表整个Windows cfg、API链接或产品运行通过。

最新原生验证须绑定修复后的提交；不将上述构建失败或本机模块通过当作全平台生产验收完成。
