//! D35 scoped 捕获的真实夹具；RED 已对旧可信视图记录缺口，当前接入生产 scoped 入口。

use super::git_metadata_budget::GitMetadataBudget;
use super::git_view::GitView;
use super::probe_budget::ProbeBudget;
use std::path::Path;

pub(super) const STATUS: &[&str] = &[
    "--no-optional-locks",
    "status",
    "--porcelain=v1",
    "-z",
    "--untracked-files=all",
];

/// 运行完整 scoped 生产准备流程，使用无损定位及已持有注册根。
/// 参数：project 是此测试约定的精确授权根；probe 是整次唯一预算。
/// 返回：真实私有视图或准备错误；不回退到旧可信实时工作树。
pub(super) fn prepare(project: &Path, probe: &mut ProbeBudget) -> Result<GitView, String> {
    let locator = diskgraph_core::QualifiedLocator::from_native_path(project).unwrap();
    let boundary = super::git_scope_boundary::GitScopeBoundary::new(project, &locator, probe)?;
    GitView::prepare_scoped(
        Path::new("git"),
        boundary,
        probe,
        GitMetadataBudget::default(),
        128 << 20,
        64 << 20,
    )
}

/// 执行实际 Git 后先显式清理，再交出输出供隔离断言；不以源复核错误代替输出证明。
/// 参数：view 为唯一视图，args 为测试固定参数，probe 为原预算。
/// 返回：真实成功 stdout；命令/清理失败会单独断言，不能伪装为隔离通过。
pub(super) fn run_and_complete(
    mut view: GitView,
    args: &[&str],
    probe: &mut ProbeBudget,
) -> Vec<u8> {
    let result = view.run(args, probe);
    let output = view
        .complete(result)
        .expect("real Git execution and cleanup");
    probe.check().unwrap();
    assert_eq!(output.exit_code, Some(0), "real Git output: {output:?}");
    output.stdout
}

/// 对 scoped 准备拒绝作断言，旧基线意外成功也先显式回收私有 owner。
/// 参数：result 为生产准备结果；返回：拒绝诊断，成功即用可读断言失败。
pub(super) fn rejection(result: Result<GitView, String>) -> String {
    match result {
        Err(error) => error,
        Ok(mut view) => {
            view.complete(Ok(())).unwrap();
            panic!("scoped capture accepted a source outside the exact authorized Git root")
        }
    }
}
