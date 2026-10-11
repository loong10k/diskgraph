# Linux 固定监督服务安装产物

当前 broker 接入 JSON `doctor` 和固定只读元数据查询：scope list/show、status、snapshots 列表、tree（无 HTML）、node、children、top、search、explore、growth、changes、explain、related、impact、candidates。新增查询的原生验收待 CI。扫描、维护、内容和文件输出命令、MCP 转发及 macOS/Windows 服务仍在实现；已有直接只读入口保持现有契约。

可信 root 安装者在隔离 Linux 环境准备全新目录，归档必须包含实际 `diskgraph`、`diskgraph-mcp`、`diskgraph-scan-worker`、`diskgraph-supervisor` 及原 worker manifest：

```sh
python3 scripts/prepare_linux_supervisor_installation.py \
  --archive /protected/diskgraph-x86_64-unknown-linux-gnu.tar.gz \
  --expected-sha256 VERIFIED_ARCHIVE_SHA256 \
  --prefix /var/lib/diskgraph-isolated \
  --service-uid 23111 --service-gid 23111 --frontend-uid 23112 --with-broker
```

服务与前端身份必须不同，首期 broker 模式只支持一个前端 UID。目录祖先必须由 root 所有且不可被组或其他用户写入；最终 socket 路径最多100个 UTF-8 字节。准备器独占创建目录，不覆盖已有安装或 ACTIVE 记录。

产物包括固定镜像、root 所有0600配置 `supervisor-config.json`、专用 `state/data`、原 bootstrap/service 槽位和未启用的 `diskgraph-supervisor.service`。准备器不调用 `systemctl`。root broker 认证前端，业务角色以独立服务 UID 运行。

前端执行 `diskgraph --json --supervisor-socket /var/lib/diskgraph-isolated/front.sock doctor`。原30秒包含连接、认证帧、输出和终态后的真实 EOF。前端结束不释放恢复容量；原监督 owner 实际退休且两个原槽变为 CLEAN 后才能重新准入。

生成的 unit 禁止自动重启；异常失败保留 ACTIVE，由显式恢复流程处理。重启进程或删除 socket 不能证明资源退休。隔离 CI 门禁检查实际安装镜像和 unit 静态语法，并在临时 Linux runner 用随机独占名称运行该生成 unit；通过原槽 FD 确认两个 CLEAN 后才正常停止。停止失败则保留 unit、安装树和原记录。静态语法检查不能代替这项运行门禁；新增运行场景尚待原生验收。本轮未安装桌面服务，生产启用仍需原生服务生命周期验收。

查询示例：`diskgraph --json --supervisor-socket /var/lib/diskgraph-isolated/front.sock scope list`。前端只转交命令声明的参数，主体与数据目录由固定安装策略决定；未知参数、全局覆盖和超过4KiB的请求在出生前拒绝。结果保留原JSON和退出码，输出最多1MiB，资源权限仍由Engine核验。

`scope show` 通过授权元数据入口读取：未授 MetadataRead、scope 撤销、原请求期限耗尽或末段能力回调超过50ms时拒绝，编码完成后的实时终检通过才交付根展示。可信内部 `Engine::scope` 保留，但不用于该公开读取入口。新增原生场景仍待 CI。
