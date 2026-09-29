# DiskGraph 依赖与许可证清单

> P0 任务 1.8（AI-01 / RE-04 / RE-05）。生成基线：`main@10149ad`，2026-09-28。
> 回归规则由 `crates/diskgraph-disktree/tests/pinned_upstream.rs` 强制执行：
> 更换 disktree pin、引入 `disktree-app`/PruneX/AgentScope 依赖、或提交生成绑定
> 都会使测试失败。数据以 `cargo metadata` 实测为准，升级依赖时同步更新本表。

## 工作区直接依赖（运行时）

| 依赖 | 版本 | 许可证 | 用途 |
| --- | --- | --- | --- |
| `disktree-core` | git `158f9cc2f0b332194a3ffc5acec47760c99146d8` | MIT | 只读扫描/树/尺寸核心；刻意不引用其 `disktree-app` 与 removal 模块 |
| `serde` / `serde_json` | 1.x | MIT OR Apache-2.0 | 快照与信封序列化 |
| `uuid` | 1.x | Apache-2.0 OR MIT | 快照 ID（v4） |
| `base64` | 0.22 | MIT OR Apache-2.0 | v2 无损 Locator 的原始字节编码 |
| `rusqlite`（bundled SQLite） | 0.40 | MIT | `diskgraph.sqlite` 持久化 |
| `sha2` | 0.10 | MIT OR Apache-2.0 | 内容确认哈希与 JWT 签名（RustCrypto；替代手写实现） |
| `hmac` | 0.12 | MIT OR Apache-2.0 | JWT HS256（RustCrypto；替代手写实现） |
| `hex` | 0.4 | MIT OR Apache-2.0 | 计划定位键与导出编码（替代手写实现） |
| `thiserror` | 2.x | MIT OR Apache-2.0 | store 错误类型 |
| `uniffi` | 0.32 | MPL-2.0 | Swift/Kotlin 绑定脚手架（构建期） |
| `rayon`（经 disktree-core 传递） | 1.12 | MIT OR Apache-2.0 | 上游并行扫描 |

## 开发/测试依赖

| 依赖 | 许可证 | 用途 |
| --- | --- | --- |
| `tempfile` | MIT OR Apache-2.0 | 隔离夹具与持久化测试 |
| `diskgraph-testkit`（本仓） | MIT | 夹具库；仅 dev-dependencies |

## 边界承诺（回归测试强制）

1. **单执行文件交付**：运行时依赖仅上述 Rust crate 与 bundled SQLite；不要求
   用户安装 Rust、disktree-app、PruneX 或任何系统命令（`ls`/`du`/`find`/`df`）。
2. **上游 pin**：`disktree-core` 固定在已审阅修订；升级必须先跑路径、权限、
   链接、占位和大小口径回归（技术方案 §1），并重新核对
   `tests/scan_fixtures.rs` 的行为断言。
3. **无 vendor/生成物入库**：不复制 DiskTree 源码；`generated-bindings/`
   等生成目录不入库。
4. **许可证合规**：全部依赖许可（MIT / Apache-2.0 / MPL-2.0）与本仓 MIT
   兼容；若未来直接移植上游代码，保留其版权与许可声明。
