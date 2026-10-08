# Git ahead/behind 单次遍历

Windows默认并发原生全量仍有5项Git探测原15秒期限失败。独立运行其中fscache真实用例通过，但耗时54.49秒（多次采样及夹具准备总时间）；这只说明失败受压力影响，不能替代默认并发验收。

现有ahead和behind分别启动rev-list并遍历相同提交图。使用完整且已校验的HEAD/upstream OID，一次 `rev-list --left-right --count HEAD...upstream` 得到左侧ahead和右侧behind，保持无网络、私有视图、原累计输出/期限/取消和终检。不得用分支名拼接参数，不改变unknown语义或压缩错误为零。严格解析一个TAB分隔的两个u64十进制数，允许一个末尾LF，拒绝空字段、额外字段、符号、嵌入换行、非法字节和溢出。

验收：原双命令路径先使单次遍历回归失败；实现后确认仅一次子进程、方向正确、原错误传播，SHA1/SHA256真实Git与两次独立计数一致，malformed/overflow/upstream/head变化原回归保持。Windows重测真实fscache与默认并发全量；仅测得实际改善才能报告性能收益，原5项失败及200k门禁不提前关闭。

## 2026-10-09 实际验证与边界

原双命令实现先得到1通过、2失败，目标失败是调用数为2以及不能拒绝不完整双字段；改为单次遍历后纯计数回归4通过。macOS Git相关组223通过、1失败、3忽略，唯一失败在扫描引擎打开时返回Unsupported，尚未进入目录替换行为；本机未配置受保护的macOS worker部署，显式worker环境变量也被平台设计拒绝，不能把该项记为通过。源码布局6通过，Engine全部目标Clippy通过。

Windows台式机真实fscache用例由54.49秒变为35.66秒，各只有一次独立观测，包含夹具和多次采样，不作为稳定性能收益或默认并发通过证明。第一次全量测试因未触达publication边界后，测试专用TLS回调保留Engine到线程析构，发生TLS AccessError并中止；新增回归先确认回调未及时释放，再改为在原断言展开时释放捕获对象，保留原缺失边界失败。该回归本机与Windows各1通过。修复后的Windows默认并发全量完整结束：665通过、30失败、4忽略，224.01秒；Clippy通过，没有再次出现TLS析构中止。期限、资源压力和20万文件300秒门禁仍开放，不以串行化、放宽期限或忽略失败替代验收。

原始Windows执行身份、源码摘要、命令、退出码、失败列表及日志摘要见 `docs/benchmarks/windows_git_single_walk_2026_10_09/`。CI 37806434084（5e87b9f5）终态13成功、10失败，完整失败日志压缩存档及摘要位于 `docs/benchmarks/ci_5e87b9f5_terminal_2026_10_09/`。除已修复的511行布局失败外，还存在Windows PowerShell Get-FileHash命令缺失、macOS stable撤权回归失败，以及Windows/macOS Intel 20万文件全流程超时，不能将此轮CI视为成功。

Windows验收脚本的独立修复：对Get-FileHash不可用的宿主增加worker副本及原生ExitProcess夹具回归，先导入模块再注入失败函数，避免自动加载覆盖测试替身。稳定红灯3通过、2失败，错误明确为Get-FileHash不可用；实现采用.NET SHA-256及有界流式读取、finally释放，保留CreateNew禁止覆盖、完整路径拒绝、源文件读锁和副本独立摘要/长度校验。Windows原生绿灯5通过（48.122秒），仍实际构建和运行两种完整32位退出码夹具。该脚本修复不代表Engine默认并发或200k容量验收通过。
