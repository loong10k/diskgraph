# Git ahead/behind 单次遍历

Windows默认并发原生全量仍有5项Git探测原15秒期限失败。独立运行其中fscache真实用例通过，但耗时54.49秒（多次采样及夹具准备总时间）；这只说明失败受压力影响，不能替代默认并发验收。

现有ahead和behind分别启动rev-list并遍历相同提交图。使用完整且已校验的HEAD/upstream OID，一次 `rev-list --left-right --count HEAD...upstream` 得到左侧ahead和右侧behind，保持无网络、私有视图、原累计输出/期限/取消和终检。不得用分支名拼接参数，不改变unknown语义或压缩错误为零。严格解析一个TAB分隔的两个u64十进制数，允许一个末尾LF，拒绝空字段、额外字段、符号、嵌入换行、非法字节和溢出。

验收：原双命令路径先使单次遍历回归失败；实现后确认仅一次子进程、方向正确、原错误传播，SHA1/SHA256真实Git与两次独立计数一致，malformed/overflow/upstream/head变化原回归保持。Windows重测真实fscache与默认并发全量；仅测得实际改善才能报告性能收益，原5项失败及200k门禁不提前关闭。

## 2026-10-09 实际验证与边界

原双命令实现先得到1通过、2失败，目标失败是调用数为2以及不能拒绝不完整双字段；改为单次遍历后纯计数回归4通过。macOS Git相关组223通过、1失败、3忽略，唯一失败在扫描引擎打开时返回Unsupported，尚未进入目录替换行为；本机未配置受保护的macOS worker部署，显式worker环境变量也被平台设计拒绝，不能把该项记为通过。源码布局6通过，Engine全部目标Clippy通过。

Windows台式机真实fscache用例由54.49秒变为35.66秒，各只有一次独立观测，包含夹具和多次采样，不作为稳定性能收益或默认并发通过证明。第一次全量测试因未触达publication边界后，测试专用TLS回调保留Engine到线程析构，发生TLS AccessError并中止；新增回归先确认回调未及时释放，再改为在原断言展开时释放捕获对象，保留原缺失边界失败。该回归本机与Windows各1通过。修复后的Windows默认并发全量完整结束：665通过、30失败、4忽略，224.01秒；Clippy通过，没有再次出现TLS析构中止。期限、资源压力和20万文件300秒门禁仍开放，不以串行化、放宽期限或忽略失败替代验收。

原始Windows执行身份、源码摘要、命令、退出码、失败列表及日志摘要见 `docs/benchmarks/windows_git_single_walk_2026_10_09/`。CI 37806434084（5e87b9f5）终态13成功、10失败，完整失败日志压缩存档及摘要位于 `docs/benchmarks/ci_5e87b9f5_terminal_2026_10_09/`。除已修复的511行布局失败外，还存在Windows PowerShell Get-FileHash命令缺失、macOS stable撤权回归失败，以及Windows/macOS Intel 20万文件全流程超时，不能将此轮CI视为成功。

Windows验收脚本的独立修复：对Get-FileHash不可用的宿主增加worker副本及原生ExitProcess夹具回归，先导入模块再注入失败函数，避免自动加载覆盖测试替身。稳定红灯3通过、2失败，错误明确为Get-FileHash不可用；实现采用.NET SHA-256及有界流式读取、finally释放，保留CreateNew禁止覆盖、完整路径拒绝、源文件读锁和副本独立摘要/长度校验。Windows原生绿灯5通过（48.122秒），仍实际构建和运行两种完整32位退出码夹具。该脚本修复不代表Engine默认并发或200k容量验收通过。

82878254推送并快进Windows项目后重新执行原生验证：Engine默认并发661通过、34失败、4忽略（269.89秒），MCP默认并发221通过（20.21秒），布局6通过，Engine/MCP全部目标Clippy通过；原15秒期限仍未满足，不把两次失败数变化解释为稳定性能结论。CLI全部目标首次125通过、1项环境失败（缺当前Cargo-built MCP companion）；构建实际当前MCP可执行文件并显式绑定后，仅重测该失败目标，1通过。Windows完整32位退出码两项在首次CLI运行已通过。证据见同目录commit_82878254_*.json，原始日志保留在台式机隔离验收目录。

最新CI为37812330459，绑定82878254；此记录时仍在运行/排队，禁止宣称全平台通过。macOS/Linux/Windows只读生产门禁保持开放，不归档当前OpenSpec变更。

## 同一私有视图的重复引用语法校验

Windows默认并发仍有Git期限失败，继续减少可证明重复的子进程：同一采样的HEAD终检仍重新读取完整commit OID、直接symbolic target、unborn存在性，并执行原元数据复核及期限/取消检查；仅对与初次成功原生check-ref-format逐字相同的分支，复用该纯语法结论。不同分支必须重新原生校验，任何OID/分支变化都拒绝；不跨请求、视图或工具缓存，不复用存在性、OID或授权结果，不自实现Git引用文法。新增回归证明同分支只校验一次、不同分支仍验证且拒绝、unborn/detached/损坏引用和原失败保持；真实Git及Windows默认并发验证前不宣称期限门禁关闭。

### stash定位三次查询合并

临时测试构建的原命令耗时观测见reference_command_phase_diagnostic.json，采集后已撤回观测源码。只读定位使用固定 `rev-parse --show-ref-format --path-format=absolute --git-common-dir --verify --quiet refs/stash^{commit}`：files后端首行和完整OID末行固定，二者之间为原始路径，不能逐行拆分丢弃内嵌换行或非UTF-8字节。原日志原生打开、对象commit验证、Git可见stash顺序、日志身份/字节与最终tip复核均保留；不合并最终tip复核，不吞掉部分输出/非零退出/取消/期限错误。纯回归先证明原三次调用失败，再验证SHA1/SHA256路径边界和严格帧；真实stash损坏、缺失、drop/expire及Windows默认15秒用例验证前不标完成。接口依据Git官方rev-parse文档（https://git-scm.com/docs/git-rev-parse），同时用当前实际Git核对组合输出。

2026-10-09候选验证：引用复用先3通过/1目标失败，再5通过；本机Git语义19、原错误3、scoped语义9通过。Windows引用不变量5、Git语义10、布局6及Clippy通过，fscache在仅引用复用候选仍超时，不宣称改善。临时按命令观测后已完整撤回git_command_context.rs的改动；观测是测试构建证据，不是生产压测。stash合并先1通过/1目标调用数失败，实现后本机3通过（真实SHA1/SHA256、macOS换行Unicode路径；Linux真实非UTF8路径待CI，纯字节解析覆盖非UTF8），原stash语义15、fscache1、Git语义19、资源3、布局6及Clippy通过。APFS拒绝创建含0xff的目录，原生非UTF8文件系统用例限定支持该命名的Linux，未跳过SHA格式验收。Windows stash定位3、原stash语义10、fscache1、布局6和Clippy通过；fscache总62.75秒为夹具及4次原默认15秒采样总耗时，仅一次观测，不是稳定加速证明。默认并发Engine全量仍需独立回归，不以这些窄测试关闭门禁。

Windows两项候选组合后的默认并发Engine全量实际650通过、53失败、4忽略（246.52秒，进程命令248.594秒）；完整原始stdout/stderr压缩归档，原文件SHA-256与源码摘要绑定在grouped_git_engine_default.json。相对前次661/34的变化不能证明性能改善或稳定回归差异，仍有真实15秒Git超时和终检预算错误；生产门禁继续开放。2026-10-08T17:30:09Z按仓库可执行文件绝对前缀检查Win32_Process，残留仓库进程0；此结果仅证明该时点没有该范围进程，不替代有限关闭和长期恢复验收。临时按命令观测源码未进入交付。
