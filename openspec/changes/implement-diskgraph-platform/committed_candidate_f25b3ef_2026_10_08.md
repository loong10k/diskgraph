# 已提交候选隔离验证

## 候选与方法

候选commit：f25b3ef2ff9f742bb355554141d5334eb3c24c1d。

通过git archive把该提交的tracked源码及测试依赖导出到独立临时目录，不复制当前工作区修改，不创建/切换分支或Git worktree。Cargo测试和Clippy工作目录均为隔离副本，复用编译缓存但由Cargo按该副本源码重新检查指纹。receipt记录导出范围、commit和关键源码Git blob身份。

原工作区Store入口存在用户未提交的reader_observation_tests模块挂载和文件，共两个测试。它们未被纳入候选，也未被删除、移动、修改或提交。先前工作区全套通过数量仍是当时实际结果，但不能用来证明已提交候选的完整测试数量。

## 结果

- Store完整测试：359通过、9忽略；忽略项不计验收。
- Engine扫描取消6、reader锁间撤权/竞争2、终态撤权见证1、终态能力14、源码规范6，均通过。
- vendored路径/pin/摘要与依赖边界4项通过；上游源码没有修改。
- Engine/Store all-targets Clippy -D warnings通过。
- 日志与receipt：docs/benchmarks/committed_candidate_f25b3ef_2026_10_08。

## 仍开放的门禁

这是当前macOS本地、已提交源码候选的证据，不是全workspace或三平台生产验收。受管worker实际安装、macOS槽位ENOENT、Windows完整负载300秒、legacy SSE失败响应及全平台同SHA门禁仍须闭环。当前远程CI 37744394886只包含5dc825d，不含本候选的后续修复。
