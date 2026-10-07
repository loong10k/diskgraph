# Unix HTTP 正常终止验收

SIGTERM/SIGINT 处理器仅记录原子停止请求。原 HTTP runtime 停止接受连接、关闭现存连接并实际 join，然后原 runner 与 Engine/Recovery/ACTIVE slot 进入已有退休路径。构造前收到请求不创建服务；构造中收到请求在 runner 出生前交还同一恢复材料。

真实 MCP 子进程回归在修复前因 SIGTERM 信号退出失败；修复后 SIGTERM、SIGINT 两项均正常退出成功。当前 macOS 本机：信号回归 2/0，HTTP 生命周期回归 8/0，源码组织门禁 11/0，MCP all-target Clippy 通过。

本机未部署受管理 worker，此证据不能证明受管理 ACTIVE→CLEAN。该路径仍须当前提交的原生 CI 验收。启动检查与信号到达不原子互斥；Pending 恢复及同步 join 不保证有限退出。SIGKILL、崩溃仍保持未确认记录，不清除或放宽容量门禁。stdio 与 Windows 信号行为不因本修复改变。
