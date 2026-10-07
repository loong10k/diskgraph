# Windows Git 源路由版本失败定位

属于 implement-diskgraph-platform / EC-02；不是监督或平台总验收完成。

当前 1c9fe464c586d2c1db203ce092d13c3d0b06a830 的 CI 37581237942，Windows Rust1.97 job112661149373，Engine套件572通过、1失败、3忽略。失败为 original_fixture_restore_retries_actual_no_delete_directory_handle。原诊断显示 attributes_recheck、directory、changed_mask0x030，仅 last_write/change_time 位不同；phaseunknown。尚不能证明变化发生于祖先还是注册叶，也不能证明原因属于扫描器或文件系统延迟。

补充仅在测试编译的 root 遍历挂载固定阶段 root_ancestor/root_leaf，复用原线程局部阶段守卫。不记录路径、身份值或时间值，不增加属性查询，不修改完整比较、打开权限、期限或恢复条件。下一次实际失败应得到准确路由类别；不允许未知阶段推断或任意错误代替目标拒绝。原生Windows结果仍待运行，当前失败不关闭。

本机 cargo fmt -p diskgraph-engine --check 和 git diff --check通过；不将macOS静态检查当作Windows运行。RED原始节选见 docs/benchmarks/supervisor_startup_admission_a08/windows_source_version_1c_ci_red.txt。

## 实际失败入口补充

29701a3 的 Windows stable job112670021363实际失败转移至 local_tracking_divergence 第一 sample_git_scoped，Engine572通过/1失败/3忽略；原目录恢复重试测试通过。错误仍为原 source changed before data access，原TLS诊断未在此次入口启用，因此尚不能判断祖先/叶目录。相应固定错误摘录见 windows_source_version_297_ci_red.txt。

现在仅在该原测试线程创建诊断guard，两个原sample错误先报告固定phase/bits，再执行原unwrap；未新增原生查询、重试或预算，不接受任意错误作为通过。macOS目标测试1/0；Windows观察仍待同SHA CI。不据这一诊断宣布竞态修复或生产就绪。
