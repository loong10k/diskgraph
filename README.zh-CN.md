<div align="center">

<img src="docs/assets/logo.svg" alt="DiskGraph" height="72" />

# DiskGraph

**面向 AI 智能体的文件关系引擎 —— 磁盘占用、文件归属、证据与历史，全部索引在本地**

`npx -y diskgraph --help` 不需要你机器上装任何东西。所有数据都留在磁盘上的一个目录里。

[English](README.md) | [简体中文](README.zh-CN.md)

[![crates.io](https://img.shields.io/badge/crates.io-diskgraph--cli-blue)](https://crates.io/crates/diskgraph-cli)
[![npm](https://img.shields.io/badge/npm-diskgraph-blue)](https://www.npmjs.com/package/diskgraph)
[![Homebrew](https://img.shields.io/badge/brew-loong10k%2Fdiskgraph%2Fdiskgraph-blue)](https://github.com/loong10k/homebrew-diskgraph)
[![License: MIT](https://img.shields.io/badge/license-MIT-green)](LICENSE)

[![macOS](https://img.shields.io/badge/macOS-支持-lightgrey)](#安装) [![Linux](https://img.shields.io/badge/Linux-支持-lightgrey)](#安装) [![Windows](https://img.shields.io/badge/Windows-支持-lightgrey)](#安装)
[![Codex](https://img.shields.io/badge/Codex-CLI-blueviolet)](#接入智能体) [![Claude Code](https://img.shields.io/badge/Claude_Code-blueviolet)](#接入智能体)

</div>

---

## 先看效果

磁盘占用是一张图，不是一个数字。三个界面从同一份索引画同一张地图，所以你在终端、浏览器、智能体回复里看到的，永远不会互相矛盾。

<table>
<tr>
<td width="62%"><img src="docs/assets/treemap-html.png" alt="DiskGraph 浏览器 treemap" /></td>
<td valign="top" width="38%">

**浏览器** —— 单个自包含文件，零 CDN、零构建：

```bash
diskgraph tree --scope <id> --html usage.html
```

点击色块下钻，点击背景返回。色相是采集器给出的分类，明暗是占父目录的比例。

</td>
</tr>
<tr>
<td valign="top" width="62%">

**终端** —— 逐层按需加载，四百万节点的索引也能秒开：

```bash
diskgraph tui --scope <id>
```

<pre>
 demo   464 MiB  10 files   ↑↓ 移动 · enter 下钻 · esc 返回 · s 排序 · q 退出
┌ disk usage · rev-ceb45ad0────────────────────────────────────────────┐┌ selection ──────┐
│ ┌atarget.bin 180 MiB───────beta 90.0 MiB──gamma 4…┐LCaches 55.0 MiB  …  ││workspaces      │
│ │                                                 │                  ││size   310 MiB  │
│ │                                                 │                  ││files  3        │
│ │                                                 │                  ││kind   directory│
│ │                                                 │                  ││category Code   │
└──────────────────────────────────────────────────┴──────────────────┘└────────────────┘
</pre>

</td>
<td valign="top" width="38%">

**智能体** —— 同一张地图的文本形态，直接出现在对话里：

```text
diskgraph_top {"scope":"…","format":"treemap"}
```

<pre>
      SIZE      TOTAL    FILES
──────────────────────────────────────────────
▎workspaces ███████████████████████████   310 MiB      3  Code
 Library    ████████▌                  55.0 MiB      1  Cache
 Media      ████▎                      25.0 MiB      1  Other

3 entries · 390 MiB in 5 files
</pre>

</td>
</tr>
</table>

## 为什么

| 你现在用的 | 它的边界 | DiskGraph |
| :--- | :--- | :--- |
| `du`、`ncdu` | 只有一棵树、没有历史、每次都要重扫 | 持久快照，能比较两个时间点 |
| Finder / 访达 | 好看，但智能体没法查询 | 同一引擎给出的有界 JSON |
| disktree | 漂亮，但只给人看 | 同样的 squarified 地图，外加 CLI / MCP / FFI |
| 给智能体一个 shell | 列目录就把上下文烧光了 | 类型化关系 + 证据 + 一次调用出图 |

DiskGraph 是**图谱，不是查看器**：每个目录都是一个带类型化关系的节点（属于某个项目、由某个工具重建、被标记为保护），每个答案都能指回产生它的证据。

## 安装

```bash
# macOS
brew install loong10k/diskgraph/diskgraph

# 任意平台，不需要工具链
npx -y diskgraph --version

# Debian / Ubuntu（从 GitHub Release 下载）
sudo dpkg -i diskgraph_0.2.1_amd64.deb

# Fedora / RHEL（从 GitHub Release 下载）
sudo dnf install diskgraph-0.2.1-1.x86_64.rpm

# Windows
winget install loong10k.DiskGraph          # 或：scoop install diskgraph

# 从源码构建
cargo install diskgraph-cli
```

CLI 与 MCP 服务端（`diskgraph-mcp`）一起安装。预编译二进制不需要 Rust；从源码构建需要 1.97+。

## 上手

```bash
# 1. 注册要观察的目录（会打印 scope id）
diskgraph scope add --root ~/projects --data-dir ~/.diskgraph

# 2. 建立索引，等扫描发布一个 revision
diskgraph index --scope <scope-id> --data-dir ~/.diskgraph --wait

# 3. 看
diskgraph top  --scope <scope-id> --data-dir ~/.diskgraph
diskgraph tree --scope <scope-id> --data-dir ~/.diskgraph --depth 4 --html usage.html
diskgraph tui  --scope <scope-id> --data-dir ~/.diskgraph
```

所有命令在 `--json` 下都返回机器可读的信封，所有错误都带稳定错误码（见 [`RELEASING.md`](RELEASING.md) 与 `diskgraph --help`）。

## 接入智能体

```bash
# Codex CLI
codex mcp add diskgraph -- diskgraph-mcp --data-dir ~/.diskgraph --profile all

# Claude Code、Cursor 或任意 MCP 宿主
claude mcp add diskgraph -- diskgraph-mcp --data-dir ~/.diskgraph --profile all
```

当前提供 18 个只读工具；profile（`read-minimal`、`read-full`、`manage`、`all`）决定列出哪些。非回环地址的 HTTP 绑定需要签名令牌，每个连接都有限制请求体大小、速率与配额。

## 实测数据

在 Apple Silicon 机器上，对真实家目录（4,526,858 个文件、观测 261 GiB）实测：

| 测量项 | |
| :--- | :--- |
| 完整索引：扫描、暂存、发布 | 4 分 17 秒 |
| 树渲染（`--depth 3`，结构化读取路径） | 10.1 秒 |
| v4 结构化列改造后的加载路径 | 快 2.4 倍 |
| 树渲染的内存占用 | 由渲染深度决定，与索引规模无关 |

这些数字来自本仓的验收记录（[`docs/acceptance/`](docs/acceptance/)），是实测而非目标或估算。

## 安全边界

- **默认只读。** 可能移动或删除文件的目录命令，在审阅界面交付之前一律返回 `unsupported`；不会悄悄启用。
- **完全本地。** 所有事情都发生在 `--data-dir`（两个 SQLite 文件）里，不向任何地方上报；生成的 HTML 报告里没有任何外部引用。
- **诚实的覆盖度。** 触发预算、深度上限或遇到不可读目录时都会明说；被截断的视图绝不会被渲染成完整视图的样子。
- **数据与控制分离。** 图谱库可重扫重建；控制库存放 scope、策略与操作历史，永不被清理流程触碰。
- **当前未签名。** 在签名配置之前，macOS Gatekeeper 与 Windows SmartScreen 会警告 —— 这是已记录的缺口，不是疏漏。

## 内部结构

十个 crate，一个引擎。`core` 持有模型、有界查询，以及三个界面共用的 squarified 布局；`store` 把快照库与控制库分开；`engine` 编排持久作业与授权面；`ops` 规划并执行需批准的受控操作；`mcp` 用三种传输讲协议；`ffi` 触达 Swift 与 Kotlin。

扫描器来自 [disktree](https://github.com/tobi/disktree)，以固定 revision 引入并在每次构建中逐文件校验摘要 —— 是复用，不是重写。

## 文档

| | |
| :--- | :--- |
| 命令与 MCP 参考 | [`docs/command-reference.md`](docs/command-reference.md) |
| 快速上手（`--help` 全文） | [`crates/diskgraph-cli/QUICKSTART.md`](crates/diskgraph-cli/QUICKSTART.md) |
| 发布渠道与操作手册 | [`RELEASING.md`](RELEASING.md) |
| 每个数字背后的验收记录 | [`docs/acceptance/`](docs/acceptance/) |
| 架构 / 技术方案 | [架构](docs/DiskGraph-Architecture.zh_CN.md) · [design](docs/DiskGraph-Technical-Design.md) |
| 需求（正式来源） | [OpenSpec 变更](openspec/changes/implement-diskgraph-platform/proposal.md) —— 14 份规格、77 条要求、121 项任务 |

## 参与贡献

欢迎 issue 与 PR。推送前请跑门禁：

```bash
scripts/gates.sh
```

## 许可证

[MIT](LICENSE)，© 2026 Loong Wan。内含 disktree-core 的 vendored 副本（MIT，© Tobi Lütke），固定在 revision `158f9cc` —— 见 [`crates/diskgraph-disktree-core/VENDORED.md`](crates/diskgraph-disktree-core/VENDORED.md)。
