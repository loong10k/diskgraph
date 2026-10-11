# Linux公开broker的实际doctor纵向链

继承PF-06、固定安装准入和原生命周期，不启用危险工具、不安装桌面服务。首个实际入口覆盖doctor；其他CLI命令和MCP必须后续沿同一原owner接入，不以本纵向链替代整体验收。

- `diskgraph-supervisor --config PATH [--once]`仅root读取预置配置，执行固定安装镜像；请求不得指定镜像、身份、目录、环境或FD。
- 配置和两镜像完整验证在绑定公开监听端、认领bootstrap和Engine出生之前完成。socket祖先root所有且不可由非root替换；已有入口拒绝，不自动删除陌生或陈旧socket。
- 固定允许UID从SO_PEERCRED绑定，逐消息SCM_CREDENTIALS一致且拒绝附加FD。首期输入为固定`DG_FRONT_INPUT\n`加原ClockStamp JSON且真实半关闭，累计最多4096字节；接收和执行沿客户端原30秒，并与broker原接收期限取交集，不能在接收结束时重新计时。只执行已有JSON doctor。
- 输出按有限帧复用原连接：一字节种类加四字节大端长度，stdout=1、stderr=2、完整i32退出码=3。正文最多4096字节，状态正文精确4字节。终态必须在业务输出实际排空后写出，随后实际关闭公开端。不能用EOF猜测成功。
- 前端验证公开socket父目录及broker原内核root身份，固定原30秒涵盖连接、输入、帧读取及输出；不得在分片后刷新。状态后额外帧、截断、未知种类或缺终态拒绝。
- ForegroundEnded仅结束公开交付，不清除槽。broker保留原监督child、私有控制和全局ACTIVE直到真实退休；错误仍保留原owner，不从exit=0推定完成。单固定服务域忙时拒绝新出生，不扩张容量。
- 独立前端二进制与实际CLI/worker在受保护隔离域验收，必须观察真实客户端wait/公共EOF、bootstrap仍ACTIVE或已由原wait合法退休、最终私有完成与原槽CLEAN。异常Pending、服务安装、多域容量、MCP和其他平台仍保持开放。
