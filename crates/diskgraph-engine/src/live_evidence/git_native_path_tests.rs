use super::git_native_path::resolve;
use std::path::Path;

#[test]
fn a_parent_step_cannot_erase_an_unverified_path_component() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    // 两者都必须拒绝；否则链接的实际父与词法父不同，缺目录也被擦掉成成功路径。
    for path in [
        "hop/../repo.git",
        "missing/../repo.git",
        "child/../../repo.git",
    ] {
        assert!(
            resolve(&base, Path::new(path)).is_err(),
            "unverified path was rewritten: {path}"
        );
    }
}

#[test]
fn leading_parents_of_a_verified_worktree_directory_remain_supported() {
    let temp = tempfile::tempdir().unwrap();
    let common = temp.path().canonicalize().unwrap().join("repo.git");
    let worktree = common.join("worktrees/id");
    std::fs::create_dir_all(&worktree).unwrap();
    assert_eq!(resolve(&worktree, Path::new("../..")).unwrap(), common);
}
