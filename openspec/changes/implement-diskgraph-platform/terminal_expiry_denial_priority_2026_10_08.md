# 终检拒权优先级的期限兼容修复

## 复查缺陷

1567618把普通reader终检SQL窗口整体取min(original deadline)，导致独立控制连接在终态回调内实际撤销scope并使数据期限过期时，原拒权被BudgetExceeded遮盖。新回归实际得到BudgetExceeded而失败。

测试最初尝试强制要求本地原生withdrawal witness，但当前夹具返回None，属于环境/前提失败，未计作行为红灯。改为实际持久scope撤销，归档红灯来自错误分类，不依赖通知能力。

## 修复与边界

终检50ms观察窗口保留原有语义。立即可取得的control guard在该窗口纯读实时拒权；发生锁竞争时，等待仍取min(原数据期限,本终检窗口)，不续期。能力明确拒绝和实时撤权仍优先。成功结果末检仍受原数据期限，过期不能交出数据。未延长消费者、重读数据或TUI准入预算。

OpenSpec对先前新增竞争场景作兼容性澄清：锁等待期限与既有负向授权观察窗口分别描述，避免把修复变成悄然改变既有错误语义。

## 验证与剩余

reader锁间撤权/竞争/期限拒权3通过，terminal_capability14通过，Engine源码规范6通过，Engine all-targets Clippy、格式与OpenSpec strict通过。日志位于docs/benchmarks/terminal_expiry_denial_priority_2026_10_08。

此改动发生在f25b3ef隔离候选验证之后，旧隔离证据不能证明本次源码；后续同SHA三平台CI必须包含本修复。整体生产就绪仍未完成。
