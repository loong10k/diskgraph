# 管理员单项授权读取优化

目标是消除 Engine 管理员权限检查每次重建全部 Grant/HashSet 的分配，保留权限能力与当前持久授权交集，不使用缓存，不替换 holds_admin 的恢复管理员规则。

新 policy_permission 在一次 SQLite 观察内读取类型化 policy(version,revoked)，并以逐行借用字段匹配精确 principal/permission/scope/current-version。无策略为None，撤销或version<=0为Some(false)；管理员伪scope不读取scope注册撤销，普通live_permission保持。

为保持旧全表异常拒绝，在有效策略时逐行借用SQLite缓冲区，检查字段类型与UTF-8，并解码i64版本。正常授权不在Rust重建所有Grant/String/HashSet。仍扫描全量grants，CPU/读取量不是O(1)，不称严格读量恒定。最初仅用typeof的候选被真实非法UTF-8 TEXT回归拒绝，未接入Engine；其旧性能记录不能用于证明当前候选。

验收：正常版本、旧/未来grant、精确身份不匹配与旧authorizer决定一致；无策略与撤销状态不误放行；独立连接撤权即时观察；无关损坏类型仍错误。release大策略配对测量只比较当前查询方法，不代替CLI/MCP端到端或原生平台验收。

首轮测试因新接口尚不存在编译失败，不记录为行为回归RED。实现后初始3项语义测试通过。Engine已接入借用实现，当前性能与独立审查证据见下文。

非法UTF-8 TEXT兼容回归确认真实RED（旧authorizer错误、新候选却允许），改为借用逐行验证后4项通过、1项手动基准忽略。基准分位数已改nearest-rank p50/p95，40样本索引19/37。

借用实现release配对测量（20,001grant、40组交替、热缓存隔离内存库）：旧重建 p50/p95 4.365/5.074 ms，新借用读取 2.605/2.995 ms。Engine管理员路径已接入；真实隔离独立连接撤销精确grant与非法UTF-8 TEXT 1项测试通过，原授权期限6项回归通过。指标不覆盖整次CLI/MCP请求或平台产物。性能数据见 admin_policy_borrowed_lookup_2026_10_07.json。

独立审查：APPROVE / CLEAR；借用验证修复了非法UTF-8回归，未改变普通scope或恢复管理员规则。

当前源码本机验证：Store 全量 lib 测试 309 通过、0 失败、3 忽略；Engine source_layout 6 通过；Store/Engine fmt 检查及 all-targets Clippy -D warnings 通过。未据此声明全平台或整个 Engine 验收通过。

## 独立读取配置成本诊断（不修改生产）

Store ignored release 诊断对隔离真实迁移库进行100样本串行配置分阶段测量；原configured与故意缺少安全配置的raw总耗时p50分别0.216958/0.214209ms。cache_size阶段p50 0.164ms，但该阶段为首次依赖schema的操作，不能把耗时全归于设置缓存参数。未删除PRAGMA、deadline或新鲜授权观察；本机空库诊断不能关闭Linux200k约1.9倍查询退化。初次并行运行互相干扰，不纳入串行结果。证据 reader_configuration_phases_2026_10_07.json。

输出字段明确限定配置值/顺序，并标记admission未测；改标签后release串行复测configured/raw p50为0.276542/0.275500ms，仍未观察到删配置收益，两次结果均保留。诊断2/0、fmt/Clippy通过，双路APPROVE/CLEAR，schema归因保持推断。

## 普通权限入口回调与控制锁分离

Engine::require 在控制锁外取得请求能力决定，然后在锁内新鲜读取持久授权交集；回调期间撤权仍必须拒绝。原require_with_control保留已有guard调用语义，其他有界终检回调仍持锁，本项不宣称全Engine回调已无锁。50ms有限重入控制读取回归旧路径实际失败，修复后应能读取持久server；独立连接在回调内撤销grant、旧能力允许时最终必须PermissionDenied。不声明同步回调硬抢占或整体期限。

普通require验证：有限重入真实RED→GREEN，相关授权组3/0；source_layout6/0，fmt/Clippy通过，双路APPROVE/CLEAR。完整Engine538/98/13，98项均Unsupported。兼容边界：控制锁中毒等错误现在在外部回调之后观察，回调可能先执行；普通入口仍可能等待控制锁，成功后的独立连接撤权仍依赖调用方后续安全门禁。

## 内容终态授权的锁外回调

require_read_terminal 保留初次范围存在/撤销检查，释放控制锁后取得ContentRead能力决定，再用新鲜控制锁核对持久交集和live_permission（无策略可信兼容模式同样拒绝scope撤销）。权限专用测试不打开文件、不替代内容平台能力门禁：原有限重入控制读失败确认RED；callback期间grant撤销、scope撤销及无策略scope撤销均需PermissionDenied。现有真实read_bounded终态测试保留；本机不支持的原生读取不得改为跳过或通过。

内容终态本机验证：权限重入真实RED→GREEN，专用两项1/0+1/0（撤销测试内3模式）；source_layout6/0，fmt/Clippy与双路APPROVE/CLEAR。完整Engine540/98/13，98项均Unsupported。实际内容能力验收保持未完成，本项未赋予平台读取能力，控制等待仍非硬有界。

## 普通权限入口固定 token expiry

require在锁外固定expiry，能力返回后、控制锁取得后及持久授权成功后检查同一到期时间；None保留可信本机兼容，不能在等待锁期间接受过期Allowed。真实锁持有直到固定expiry之后、能力决定在expiry之前取得的回归，旧路径返回成功为RED，修复须PermissionDenied。复用scope列表已有相同expiry门禁；不刷新请求deadline，不承诺控制锁硬期限。已过期能力优先拒绝，不等待控制错误或读取策略诊断。

固定expiry本机验证：实际锁竞争旧路径Ok真实RED→GREEN，权限组4/0、scope组7/0，source_layout6/0、fmt/Clippy及双路APPROVE/CLEAR。完整Engine548/98/13，98项均Unsupported。错误优先限定：首次expiry检查已发现到期时不获取控制锁；锁获取或授权SQL自身失败仍保留原错误，不保证所有并发到期覆盖其他错误。Unix秒墙钟不是单调执行预算。

## 内容终态固定 token expiry

require_read_terminal在所有控制锁外捕获一次固定expiry，范围观察后、能力返回后、最终锁取得后和SQL授权成功后共用到期门禁。已到期则PermissionDenied，不返回读取body/partial；None本机模式保持。实际固定expiry、能力取得时仍live但回调返回时已到期的权限专用测试旧路径Ok为RED，修复须拒权；不打开文件、不替代原生内容门禁。锁/SQL自身错误保留，不提供同步硬抢占或有限退出证明。

内容expiry本机验证：固定expiry回归旧Ok真实RED→GREEN，permission-only两项2/0，重入1/0，source_layout6/0、fmt/Clippy与双路APPROVE/CLEAR。完整Engine549/98/13，98项均Unsupported。权限专用测试没有打开实际文件，不构成原生内容验收。

## Revision 初始授权固定 token expiry

authorize_revision_owner_until 与 require_reader_capability_until 复用固定请求到期时间，trusted reader 在获取控制锁前捕获并作为参数传入 helper；初始观察前、能力回调前后及最终持久授权成功后拒绝过期能力。None 保留可信本机语义；不刷新请求执行期限。既有控制锁内的回调尚未移出，本项不声称解决回调重入、硬抢占或有限退出。

回归 initial_revision_authorization_rejects_expired_allowed_capability 使用已发布的隔离 metadata 夹具和明确到期但返回 Allowed 的 Authorizer：旧 owner 路径实际返回 Ok 为 RED；修复后 owner 和 trusted reader 两条路径均 PermissionDenied。权限组本机 5/0；尚未完成本批完整回归、双路审查及三平台 CI，不标记平台验收完成。

本批追加验证：真实能力回调跨到期时间的 owner/reader 两路径拒权通过；公开 reader 的 expiry getter 有限控制锁重入通过。完整 Engine 552/98/13，98 个失败均 Unsupported，不视为平台通过；Clippy 通过。末段 expiry 全覆盖及 decide 持 mutex 仍未闭环。

## Trusted revision reader 终态固定 expiry

with_authorized_revision_reader 在 consumer 成功返回后、终检控制锁取得后、终检能力/实时授权成功后及归属检查完成后复核初始锁外捕获的同一 expiry。到期不提交结果，None 兼容不变，原 consumer/SQL/锁错误保留。真实 consumer 在 expiry 前开始、expiry 后结束且原执行预算仍有效的回归，旧代码 Ok 为 RED，要求 PermissionDenied。此项不关闭 display reader 或其他 reader 的全部终检差距，不保证控制锁及同步回调硬抢占。

终检 expiry 本机验证：旧 Ok 实际 RED；权限组8/0、完整Engine553/98/13（98项均Unsupported），source_layout6/0、fmt/Clippy与双路APPROVE/CLEAR。仍不构成平台通过。

## Display reader 固定 expiry

导航和截断画布共用初始锁外固定 expiry，在准备前、初始授权后、消费后终检阶段、能力回调后及返回前检查；过期请求不可进入画布消费者，Truncated 不绕过到期。旧代码过期 Allowed 返回 Ok 的真实回归为 RED。保持已有消费错误经终检、归属及撤权优先规则，不刷新执行预算。不声称同步回调锁隔离或三平台完成。

Display本机验证：预先到期真实RED→GREEN；消费期间到期Truncated与Complete分别通过（原执行预算仍有效）。完整Engine555/98/13于Complete补测加入前执行，随后Complete精确测试1/0；98失败均Unsupported。source_layout6/0、fmt/Clippy与双路APPROVE/CLEAR。本项不冒充最新完整或平台验收。

## Revision owner 初始能力回调锁隔离

authorize_revision_owner_until 初始归属检查后释放控制锁，再执行宿主 decide；随后沿原请求期限重新获取锁，重验 server 身份、实时策略与 scope 撤销。固定 expiry 不变。有限 50ms callback 控制锁重入在旧实现失败为 RED；允许及 callback 内撤销 grant 的两种结果分别要求成功和 PermissionDenied。可信 reader/display 的其他回调仍不据此声明锁隔离完成。

Owner回调锁隔离验收：旧有限重入真实RED→GREEN；权限组12/0，完整Engine557/98/13（98均Unsupported），source_layout6/0、fmt/Clippy及双路APPROVE/CLEAR。仍不视为平台通过。
