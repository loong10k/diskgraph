//! 真实旧执行器的RED桥；只绑定受管理宿主，不替换出生/清理或伪造恢复成功。
use super::ProbeLimits;
use super::git_private_directory::GitPrivateDirectory;
use super::probe_budget::ProbeBudget;
use super::probe_directory_witness::ProbeDirectoryWitness;
use super::probe_execution::run_probe;
use crate::ProbeHost;
use std::process::Command;
use std::sync::Arc;

/// 真实创建、写入和正常complete正控；失败在出生前显式报告，不算owner恢复RED。
pub(crate) fn qualify_private_probe_directory_for_test(
    limits: &ProbeLimits,
    host: &ProbeHost,
) -> Result<(), String> {
    let mut budget = ProbeBudget::new(limits).map_err(|error| error.to_string())?;
    budget
        .bind_probe_host(Arc::clone(&host.registry))
        .map_err(|error| error.to_string())?;
    let mut directory = GitPrivateDirectory::new(&mut budget)?;
    directory.write(
        &directory.path().join("probe-input"),
        b"private probe input",
        &mut budget,
    )?;
    let witness = ProbeDirectoryWitness::capture(&directory)?;
    if !witness.same_directory_exists()? {
        return Err("real Git private directory identity absent".into());
    }
    directory.complete(Ok(()))?;
    if !witness.absent()? {
        return Err("normal Git private complete did not remove its root".into());
    }
    Ok(())
}

/// 私有目录在生产complete/Drop边界结束，真实child以此目录为cwd；不伪造目录保留或恢复。
pub(crate) fn run_managed_private_probe_for_test(
    command: &mut Command,
    limits: &ProbeLimits,
    host: &ProbeHost,
    witness: impl FnOnce(ProbeDirectoryWitness),
) -> Result<(), String> {
    let mut budget = ProbeBudget::new(limits).map_err(|error| error.to_string())?;
    budget
        .bind_probe_host(Arc::clone(&host.registry))
        .map_err(|error| error.to_string())?;
    let mut directory = GitPrivateDirectory::new(&mut budget)
        .map_err(|error| format!("private Git creation before native birth: {error}"))?;
    directory.write(
        &directory.path().join("probe-input"),
        b"private probe input",
        &mut budget,
    )?;
    let observed = ProbeDirectoryWitness::capture(&directory)?;
    if !observed.same_directory_exists()? {
        return Err("private Git initial identity absent".into());
    }
    command
        .current_dir(directory.path())
        .env("DG_CONTROL_DIR", directory.path());
    witness(observed);
    let result = run_probe(command, &mut budget)
        .map(|_| ())
        .map_err(|error| error.to_string());
    directory.complete(result)
}

/// 测试受管理请求走当前真实run_probe；错误只投影已有诊断，panic原值不捕获。
pub(crate) fn run_managed_probe_for_test(
    command: &mut Command,
    limits: &ProbeLimits,
    host: &ProbeHost,
) -> Result<(), String> {
    let mut budget = ProbeBudget::new(limits).map_err(|error| error.to_string())?;
    budget
        .bind_probe_host(Arc::clone(&host.registry))
        .map_err(|error| error.to_string())?;
    run_probe(command, &mut budget)
        .map(|_| ())
        .map_err(|error| error.to_string())
}
