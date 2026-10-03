//! cleanup：既有文件操作职责的原生 Rust 实现。
use crate::OpsError;
use crate::docker::cleanup_result::CleanupResult;
use crate::docker::command::run;
use crate::docker::docker_object::DockerObject;
use crate::docker::usage_check::UsageCheck;
use crate::docker::usage_check::usage_check;
use crate::specialist::CommandRunner;
use crate::specialist::SpecialistVerdict;
use crate::specialist::verify_specialist_result;
use std::path::Path;

/// 构造确切对象的删除参数。
/// 参数：object 为已批准 Docker 对象。
/// 返回：已支持类型的结构化参数 Some；未知类型返回 None。
/// The removal command for one object: exact id, no wildcards, no prune
/// anywhere in the vocabulary.
pub(super) fn removal_args(object: &DockerObject) -> Option<Vec<&'static str>> {
    match object.kind {
        "volume" => Some(vec!["volume", "rm"]),
        "image" => Some(vec!["rmi"]),
        "container" => Some(vec!["rm"]),
        _ => None,
    }
}

/// 调用工具清理单个已批准确切对象。
/// 参数：runner 为执行器；docker 为固定路径；objects 为可信调用者提供的已批准对象列表。
/// 返回：与逐项执行对应的 CleanupResult 列表或执行错误；不构造全局 prune。
/// Removes exactly the approved objects, one command per object, each preceded
/// by a fresh usage check and followed by a check that Docker really forgot
/// the object. There is no batch mode and no prune flag anywhere in this path.
pub fn docker_cleanup(
    runner: &dyn CommandRunner,
    docker: &Path,
    objects: &[DockerObject],
) -> Result<Vec<CleanupResult>, OpsError> {
    let mut results = Vec::new();
    for object in objects {
        match usage_check(runner, docker, object)? {
            UsageCheck::InUse { reason } => {
                results.push(CleanupResult::SkippedInUse { reason });
                continue;
            }
            UsageCheck::Unused => {}
        }
        let Some(prefix) = removal_args(object) else {
            results.push(CleanupResult::NeedsAttention {
                reason: format!("no removal command is defined for kind {}", object.kind),
            });
            continue;
        };
        let mut args = prefix;
        args.push(&object.id);
        let outcome = run(runner, docker, &args)?;
        // The effect is proven by Docker forgetting the object, not by an
        // exit code alone.
        let forgotten = run(runner, docker, &["inspect", &object.id])?;
        if forgotten.exit_code != 0 {
            results.push(CleanupResult::Confirmed);
            continue;
        }
        // Docker still knows the object: verify the command output to decide
        // whether that is a wait-and-retry or a real failure.
        results.push(match verify_specialist_result(&outcome, &object.id) {
            SpecialistVerdict::Confirmed => CleanupResult::Confirmed,
            SpecialistVerdict::NeedsAttention { reason } => {
                CleanupResult::NeedsAttention { reason }
            }
        });
    }
    Ok(results)
}
