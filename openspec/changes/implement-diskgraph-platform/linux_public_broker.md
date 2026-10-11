# Linux公开broker的只读CLI纵向链

继承PF-06、固定安装准入和原生命周期，不启用危险工具、不安装桌面服务。首个原生验收入口覆盖doctor；本次将限定的只读元数据查询接入同一原owner，原生查询与安装运行验收待新CI。其他CLI命令和MCP仍需继续接入，不以本纵向链替代整体验收。

- `diskgraph-supervisor --config PATH [--once]`仅root读取预置配置，执行固定安装镜像；请求不得指定镜像、身份、目录、环境或FD。
- 配置和两镜像完整验证在绑定公开监听端、认领bootstrap和Engine出生之前完成。socket祖先root所有且不可由非root替换；已有入口拒绝，不自动删除陌生或陈旧socket。
- 固定允许UID从SO_PEERCRED绑定，逐消息SCM_CREDENTIALS一致且拒绝附加FD。首期输入为固定`DG_FRONT_INPUT\n`加原ClockStamp JSON且真实半关闭，累计最多4096字节；接收和执行沿客户端原30秒，并与broker原接收期限取交集，不能在接收结束时重新计时。旧协议只执行已有JSON doctor；新增版本化协议按下文固定查询集合分发。
- 输出按有限帧复用原连接：一字节种类加四字节大端长度，stdout=1、stderr=2、完整i32退出码=3。正文最多4096字节，状态正文精确4字节。终态必须在业务输出实际排空后写出，随后实际关闭公开端。不能用EOF猜测成功。
- 前端验证公开socket父目录及broker原内核root身份，固定原30秒涵盖连接、输入、帧读取及输出；不得在分片后刷新。状态后额外帧、截断、未知种类或缺终态拒绝。
- ForegroundEnded仅结束公开交付，不清除槽。broker保留原监督child、私有控制和全局ACTIVE直到真实退休；错误仍保留原owner，不从exit=0推定完成。单固定服务域忙时拒绝新出生，不扩张容量。
- 独立前端二进制与实际CLI/worker在受保护隔离域验收，必须观察真实客户端wait/公共EOF、bootstrap仍ACTIVE或已由原wait合法退休、最终私有完成与原槽CLEAN。异常Pending、服务安装、多域容量、MCP和其他平台仍保持开放。

## 固定安装产物验收

现有隔离安装准备器增加显式`--with-broker`：旧三镜像准备路径保持兼容；启用该模式必须包含第四个原生broker镜像，且首期恰有一个前端UID。所有配置检查在创建安装树前完成，输出root所有0600配置、root保护socket位置、state/data专用服务目录及未启用的systemd unit。配置摘要取实际独占复制的镜像，不能引用包内自签摘要。unit执行固定broker及配置，异常退出不自动重启或删除ACTIVE；准备器不得调用systemctl或安装桌面服务。隔离原生验收须用实际打包的四镜像经准备器启动broker，检查实际CLI结果、原owner退休及新安装tree重复准备拒绝。三平台服务父项保持开放。

Linux GNU安装归档新增固定broker镜像，实际构建、入包、解包三阶段均检查原glibc2.17基线与镜像摘要；缺少broker或其ABI超标必须在写归档、执行程序前拒绝。非Linux包保持现有镜像集合，不能把Linux服务产物当作其他平台实现。

隔离CI的systemd运行场景：只在明确root隔离验收开关下，将生成unit以随机独占名称写入临时runner的/run/systemd/system，启动实际固定broker并以独立前端UID调用doctor；须从始终持有的原槽FD观察两个CLEAN，再正常停止该唯一unit。失败处置不算原owner退休，移除临时unit不算恢复。此场景不在桌面运行，不启用开机启动，不能覆盖实际Pending与异常I/O验收。

## 只读元数据查询转交

固定JSON doctor兼容协议继续保留。新增版本化只读查询请求，包含原ClockStamp和不可变受限命令描述：scope list/show、status、snapshots列表、tree（不写HTML）、node、children、top、search、explore、growth、changes、explain、related、impact、candidates。只接受各命令声明的必要/可选参数及布尔开关，数字字段在出生前验证，重复参数、未知命令/字段、全局数据目录/主体/环境/FD覆盖及过大正文拒绝。参数值以单一`--flag=value`传递，不能被Clap解释成另一条全局开关；scope/revision真实授权仍由现有Engine入口完成。原请求最多4096字节、输出最多1MiB、原30秒不刷新。当前每安装域仍只允许一个可信本地前端UID并使用既有local-user策略；不把这个本地模式用于远程MCP身份。实际CLI查询/业务失败必须保持JSON和原退出状态，之后原owner独立退休。扫描、维护、HTML文件交付和MCP转交仍需单独落地，不能据本增量勾选整个CLI/MCP父项。

版本化请求原生负控：同一真实Linux时间域/有效25秒ClockStamp首先成功执行doctor作为正控，分别仅改变全局主体覆盖、未知命令、重复scope、非法数字、写命令或顶层环境字段。每个负例须观察原状态10与真实公开EOF，数据库未出生、两个原槽仍CLEAN且原broker正常wait；不得用过期/错域时钟替代命令校验。夹具逐笔读取扣同一原30秒，不刷新分片时间；此新增门禁尚待原生执行。

CLI scope show补充统一授权验收：未知主体不可读取注册根展示；正确授权保留既有字段；原dispatch期限耗尽拒绝；编码后能力回调或持久撤权拒绝且不交付缓冲结果。不得以可信内部scope(scope_id)读取替代对外授权。原store/Engine可信兼容读取保留。
