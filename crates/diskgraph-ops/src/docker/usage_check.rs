//! usage_check：既有文件操作职责的原生 Rust 实现。
use crate::OpsError;
use crate::docker::command::run;
use crate::docker::docker_object::DockerObject;
use crate::specialist::CommandRunner;
use std::path::Path;

/// Docker 对象未使用或使用中/无法确认的保守结果。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::docker::UsageCheck`，保留既有语义。
/// The live usage check one object must pass immediately before its removal
/// command runs (7.9, OP-07). Anything in use is refused, never force-removed.
pub enum UsageCheck {
    /// Nothing Docker knows of is using the object right now.
    Unused,
    /// A live dependency names the object; the cleanup must skip it.
    InUse { reason: String },
}

/// 查询确切 Docker 对象是否正在使用。
/// 参数：runner、docker 和 object 指定工具与确切对象。
/// 返回：确认、使用中或未知结果；未知不允许视作可清理。
pub fn usage_check(
    runner: &dyn CommandRunner,
    docker: &Path,
    object: &DockerObject,
) -> Result<UsageCheck, OpsError> {
    match object.kind {
        "container" => {
            let state = run(
                runner,
                docker,
                &["inspect", "-f", "{{.State.Running}}", &object.id],
            )?;
            if state.exit_code != 0 {
                // Unknown state blocks: a container we cannot inspect is not a
                // container we may remove.
                return Ok(UsageCheck::InUse {
                    reason: format!(
                        "container {} could not be inspected (exit {})",
                        object.id, state.exit_code
                    ),
                });
            }
            let running = String::from_utf8_lossy(&state.stdout).trim() == "true";
            if running {
                Ok(UsageCheck::InUse {
                    reason: format!("container {} is running", object.id),
                })
            } else {
                Ok(UsageCheck::Unused)
            }
        }
        "volume" => {
            let users = run(
                runner,
                docker,
                &[
                    "ps",
                    "-a",
                    "--filter",
                    &format!("volume={}", object.id),
                    "-q",
                ],
            )?;
            let users = String::from_utf8_lossy(&users.stdout);
            let users: Vec<&str> = users
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect();
            if users.is_empty() {
                Ok(UsageCheck::Unused)
            } else {
                Ok(UsageCheck::InUse {
                    reason: format!(
                        "volume {} is used by container(s) {}",
                        object.id,
                        users.join(", ")
                    ),
                })
            }
        }
        "image" => {
            let users = run(
                runner,
                docker,
                &[
                    "ps",
                    "-a",
                    "--filter",
                    &format!("ancestor={}", object.id),
                    "-q",
                ],
            )?;
            let users = String::from_utf8_lossy(&users.stdout);
            let users: Vec<&str> = users
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect();
            if users.is_empty() {
                Ok(UsageCheck::Unused)
            } else {
                Ok(UsageCheck::InUse {
                    reason: format!(
                        "image {} backs container(s) {}",
                        object.id,
                        users.join(", ")
                    ),
                })
            }
        }
        other => Ok(UsageCheck::InUse {
            reason: format!("no usage check is defined for kind {other}; refusing"),
        }),
    }
}
