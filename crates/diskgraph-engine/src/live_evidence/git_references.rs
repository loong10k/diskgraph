//! 用 Git 原生引用接口区分缺失、损坏与命令错误；不从错误文本推断 unborn。

use super::git_output::{line, object_id, successful};
use super::probe_output::ProbeOutput;

/// 观察 HEAD 的 commit OID 与当前分支。来源：原生 Rust Git 引用采样。
/// 参数：run 是借用整次执行预算的固定参数执行入口。
/// 返回：已验证的 OID/分支；只有明确缺少当前分支才返回无 OID。
pub(super) fn head(
    run: &mut impl FnMut(&[&str]) -> Result<ProbeOutput, String>,
) -> Result<(Option<String>, Option<String>), String> {
    let output = run(&["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])?;
    let oid = if output.exit_code == Some(1) && output.stdout.is_empty() && output.stderr.is_empty()
    {
        None
    } else {
        Some(object_id(&successful(output)?)?)
    };
    // 保留 HEAD 的直接目标，否则已有但悬空的 symbolic branch 会被递归成缺分支。
    let symbolic = run(&["symbolic-ref", "--quiet", "--no-recurse", "HEAD"])?;
    let branch = if symbolic.exit_code == Some(1)
        && symbolic.stdout.is_empty()
        && symbolic.stderr.is_empty()
    {
        None
    } else {
        let branch = line(&successful(symbolic)?)?.to_owned();
        if !branch.starts_with("refs/heads/") {
            return Err("HEAD does not name a supported local branch".into());
        }
        validate_name(run, &branch)?;
        Some(branch)
    };
    if oid.is_none() {
        let branch = branch.as_deref().ok_or("unresolvable detached HEAD")?;
        if exists(run, branch)? {
            return Err("HEAD reference exists but does not resolve to a commit".into());
        }
    }
    Ok((oid, branch))
}

/// 查询已存在分支的本地 upstream commit。来源：原生 Rust Git 引用采样。
/// 参数：run 复用整次预算，branch/head_oid 为已验证的当前分支与 commit。
/// 返回：本地跟踪 OID及映射是否存在；损坏引用、格式或采样变化返回错误。
pub(super) fn upstream(
    run: &mut impl FnMut(&[&str]) -> Result<ProbeOutput, String>,
    branch: &str,
    head_oid: &str,
) -> Result<(Option<String>, bool), String> {
    let bytes = successful(run(&[
        "for-each-ref",
        "--format=%(refname)%00%(objectname)%00%(upstream)%00",
        "--",
        branch,
    ])?)?;
    let record = bytes.strip_suffix(b"\n").ok_or("invalid upstream record")?;
    let fields: Vec<&[u8]> = record.splitn(4, |byte| *byte == 0).collect();
    if fields.len() != 4 || !fields[3].is_empty() || fields[0] != branch.as_bytes() {
        return Err("invalid upstream record".into());
    }
    if object_id(fields[1])? != head_oid {
        return Err("HEAD reference changed during sampling or is not a commit".into());
    }
    if fields[2].is_empty() {
        return Ok((None, false));
    }
    let name = line(fields[2])?;
    validate_name(run, name)?;
    if !exists(run, name)? {
        return Ok((None, true));
    }
    let oid = object_id(&successful(run(&[
        "rev-parse",
        "--verify",
        "--quiet",
        &format!("{name}^{{commit}}"),
    ])?)?)?;
    Ok((Some(oid), true))
}

/// 明确验证引用存在性，不能把损坏引用解释成不存在。
/// 参数：run 复用预算，name 为固定或已校验的完整引用名。
/// 返回：存在为 true、明确缺失为 false；Git 不支持该接口或查询失败报错。
pub(super) fn exists(
    run: &mut impl FnMut(&[&str]) -> Result<ProbeOutput, String>,
    name: &str,
) -> Result<bool, String> {
    // Git 2.46+ 的 --exists 把缺失(2)与其他错误(1)分开。
    // for-each-ref 会安静跳过悬空 symbolic ref，不能用于证明不存在。
    let output = run(&["show-ref", "--exists", name])?;
    match output.exit_code {
        Some(0) if output.stdout.is_empty() && output.stderr.is_empty() => Ok(true),
        Some(2) if output.stdout.is_empty() => Ok(false),
        _ => Err(format!(
            "reference existence query failed (Git 2.46+ required): {}",
            successful(output)
                .err()
                .unwrap_or_else(|| "invalid reference response".into()),
        )),
    }
}

fn validate_name(
    run: &mut impl FnMut(&[&str]) -> Result<ProbeOutput, String>,
    name: &str,
) -> Result<(), String> {
    if !name.starts_with("refs/") {
        return Err("invalid complete Git reference name".into());
    }
    let result = successful(run(&["check-ref-format", name])?)?;
    if !result.is_empty() {
        return Err("invalid reference validation response".into());
    }
    Ok(())
}
