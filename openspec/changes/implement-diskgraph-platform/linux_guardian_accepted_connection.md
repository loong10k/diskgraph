# PF-06 原连接 guardian 与客户 executor 增量

延续已批准的专用 service UID guardian＋真实客户 OS 凭据 executor 合同，不缩减完整只读 CLI/MCP 或三平台验收。

## 可观察行为

- 固定 `--connection` 入口仅接受 stdin 上的原 AF_UNIX STREAM。监听者必须先于客户发送启用 SO_PASSCRED；实际客户 UID/GID/完整辅助组来自同一内核连接，非请求正文。
- root 启动层只打开受信配置、原镜像及容量槽。客户 executor 不获得 root 目录、namespace 或容量 FD，原 Engine/pending 资源始终留在客户 executor。
- 原 guardian 在认证出生后，永久变为配置中的专用 UID/GID；三组身份一致、辅助组清空、能力全清、NNP=1、dumpable=0。降权失败禁止公开结果交付，原 owner/ACTIVE 保留。
- 客户数据库由客户 UID 创建。代码镜像对客户只可执行，不可读取。旧 service slot 不被客户 executor 使用。
- 公开 EOF/超时不能释放容量或污染私有恢复资格。只有认证完成和原 child 实际 wait 才允许原 bootstrap CLEAN。
- 运行期限与启动期限继续按原合同分别处理；本增量不增加扫描时间上限。
- 新安装模式 `--with-guardian --frontend-gid GID` 与旧 broker 模式互斥，独占创建新目录。数据目录实际 owner 为客户 UID/GID、0700；CLI/MCP/worker 镜像为 root 0511。安装 receipt 记录真实 fstat 身份，不凭期望值声称已安装。
- socket activation 采用原 accepted stdin、SO_PASSCRED 和最多四连接；监听 socket 只允许配置客户访问。模板只向原 guardian 发 SIGTERM、不对原 child 强制 SIGKILL；停止期限不替代资源退休。
- guardian 的 SIGTERM/SIGINT handler 只锁存原进程 atomic 停止标记。工作出生前取消准入；已收到请求后的认证出生按原有限期限完成，停止公开交付后继续同一原私有恢复。

```mermaid
flowchart LR
    U[客户 CLI] --> SO[原 Unix socket\n内核身份与完整辅助组]
    SO --> R[root 启动层\n固定配置、镜像、原容量槽]
    R --> X[客户 executor\n原 Engine、扫描及恢复资源]
    R --> G[同一 guardian\n永久专用 service UID]
    X -->|认证私有完成| G
    G --> W[原 child 实际 wait]
    W --> C[原容量槽 CLEAN]
    STOP[SIGTERM / 公开断开] --> P[停止公开交付]
    P --> G
```

systemd 属性依据官方 [socket contract](https://raw.githubusercontent.com/systemd/systemd/main/man/systemd.socket.xml)、[stdin contract](https://raw.githubusercontent.com/systemd/systemd/main/man/systemd.exec.xml) 和 [kill contract](https://raw.githubusercontent.com/systemd/systemd/main/man/systemd.kill.xml)。静态产物检查不能代替实际 unit 启动/停止。

## 验收与实际边界

- 隔离 Linux ARM64 C 层真实身份切换四例已通过：合法专用身份、无效 UID、无效 GID、非 root 调用。合法例实际恢复 root 返回 EPERM。
- 严格 ARM64/x86_64 C 编译通过；x86_64 运行未由本机证明。
- 本机 Rust 源码门禁 6/6，通过 macOS all-target Clippy；Linux 条件编译主体不被这些结果覆盖。
- `scripts/qualify_linux_guardian.py` 使用原 accepted socket、真实客户辅助组、当前 CLI/worker/supervisor 产物，要求公开 doctor 完成、客户 UID 数据库、原 guardian 正常 wait、同一 bootstrap CLEAN；另验客户发完同一请求后立即断开，仍须原正常 wait 后 CLEAN。尚待原生 CI，不声称已有运行红灯或绿灯。
- 本增量尚不证明扫描/完整查询、MCP、systemd 生命周期、LSM 上下文保真、macOS/Windows 服务或生产就绪。父任务不勾选。
- 前序源码 eb80d492 的 CI 38195149112 已终态成功，23/23 job 和五种原生包通过，原始 run 保留于 `docs/benchmarks/ci_eb80d492_terminal_2026_10_12/`。它未包含本 guardian 候选，不能替代新运行验收。
- 安装增量 TDD：三项新测试在旧 API 缺少参数/产物生成器时出现五个 API 错误，并非原生行为红灯；另一个 ListenStream 原路径测试复现 ExecStart 引号误用的行为失败，修复后 4/4 通过，旧安装回归 19/19 通过。ListenStream 原值经 unit_path_printf/socket_address_parse 解析，不能套用参数引号，依据 [systemd parser](https://raw.githubusercontent.com/systemd/systemd/main/src/core/load-fragment.c)。原生夹具已改为真正经过四镜像归档/独占安装器，并加入工作出生前的真实 SIGTERM/公开 EOF/数据库未出生检查；仍待新源码 CI，不声明已通过。
