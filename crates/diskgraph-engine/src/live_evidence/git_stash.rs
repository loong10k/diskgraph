//! 对 files 引用后端核对原始 stash reflog；Git list 的成功退出不足以证明完整性。

use super::git_output::{object_id, stash_count, successful};
use super::git_reflog_file::GitReflogFile;
use super::probe_budget::ProbeBudget;
use super::probe_output::ProbeOutput;
use std::path::PathBuf;

/// 核对 files 后端的完整 stash reflog、对象类型及 Git 可见顺序。
/// 来源：原生 Rust diskgraph-engine::live_evidence::git_stash 与 Git files reflog 格式。
/// 参数：run 为借用同一预算的固定 Git 命令入口，budget 为整次采样共享期限与字节额度。
/// 返回：完整可确认的 stash 数量；缺损、并发变化、不支持的后端或资源超限均返回错误。
pub(super) fn count(
    run: &mut impl FnMut(&[&str], &mut ProbeBudget) -> Result<ProbeOutput, String>,
    budget: &mut ProbeBudget,
) -> Result<u64, String> {
    budget.check().map_err(|error| error.to_string())?;
    let backend = successful(run(&["rev-parse", "--show-ref-format"], budget)?)?;
    if backend != b"files\n" {
        return Err("unsupported Git reference backend for complete stash count".into());
    }
    let tip_before = stash_tip(run, budget)?;
    // Git 会 canonicalize --git-path 的完整结果，掩盖日志路径中的链接；只让 Git 定位 common 根。
    let raw_common = successful(run(
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        budget,
    )?)?;
    let path = log_path(&raw_common)?;
    let mut log = GitReflogFile::open(&path, budget)?;
    let first = match log.as_mut() {
        Some(log) => log.read_bounded(budget)?,
        None => Vec::new(),
    };
    let entries = parse_log(&first)?;
    for oid in &entries {
        if oid.len() != tip_before.len() {
            return Err("stash reflog object format disagrees with current tip".into());
        }
        let kind = successful(run(&["cat-file", "-t", oid], budget)?)?;
        if kind != b"commit\n" {
            return Err("stash reflog names a non-commit object".into());
        }
    }
    let listed = successful(run(&["stash", "list", "--format=%H"], budget)?)?;
    let listed_count = stash_count(&listed)?;
    if listed_count != u64::try_from(entries.len()).map_err(|_| "stash count overflow")?
        || !entries
            .iter()
            .rev()
            .zip(
                listed
                    .split_inclusive(|byte| *byte == b'\n')
                    .map(|line| &line[..line.len() - 1]),
            )
            .all(|(raw, visible)| raw.as_bytes() == visible)
    {
        return Err("stash reflog and Git list disagree".into());
    }
    // 二次按原路径安全打开并比较字节，能识别替换和同长度改写；这不是 Git 原子快照。
    let stable_log = match log.as_mut() {
        Some(log) => log.matches_path(&path, budget)? && log.read_bounded(budget)? == first,
        None => GitReflogFile::open(&path, budget)?.is_none(),
    };
    if !stable_log || stash_tip(run, budget)? != tip_before {
        return Err("stash changed during sampling".into());
    }
    budget.check().map_err(|error| error.to_string())?;
    u64::try_from(entries.len()).map_err(|_| "stash count overflow".into())
}

fn stash_tip(
    run: &mut impl FnMut(&[&str], &mut ProbeBudget) -> Result<ProbeOutput, String>,
    budget: &mut ProbeBudget,
) -> Result<String, String> {
    object_id(&successful(run(
        &["rev-parse", "--verify", "--quiet", "refs/stash^{commit}"],
        budget,
    )?)?)
}

fn log_path(bytes: &[u8]) -> Result<PathBuf, String> {
    let raw = bytes
        .strip_suffix(b"\n")
        .ok_or("invalid Git common directory")?;
    if raw.is_empty() || raw.contains(&0) {
        return Err("invalid Git common directory".into());
    }
    #[cfg(unix)]
    let path = {
        use std::os::unix::ffi::OsStringExt;
        PathBuf::from(std::ffi::OsString::from_vec(raw.to_vec()))
    };
    #[cfg(windows)]
    let path = PathBuf::from(std::str::from_utf8(raw).map_err(|_| "stash path is not UTF-8")?);
    #[cfg(not(any(unix, windows)))]
    return Err("unsupported stash reflog platform".into());
    if !path.is_absolute() {
        return Err("Git common directory is not absolute".into());
    }
    Ok(path.join("logs").join("refs").join("stash"))
}

fn parse_log(bytes: &[u8]) -> Result<Vec<String>, String> {
    let mut entries = Vec::new();
    if bytes.is_empty() {
        return Ok(entries);
    }
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        let record = line
            .strip_suffix(b"\n")
            .ok_or("unterminated stash reflog record")?;
        entries.push(parse_record(record)?);
    }
    Ok(entries)
}

fn parse_record(record: &[u8]) -> Result<String, String> {
    let bad = || "invalid stash reflog record".to_owned();
    let width = record
        .iter()
        .position(|byte| *byte == b' ')
        .ok_or_else(bad)?;
    if !matches!(width, 40 | 64) || record.len() < 2 * width + 3 {
        return Err(bad());
    }
    let old = &record[..width];
    let new = &record[width + 1..2 * width + 1];
    if !old.iter().all(u8::is_ascii_hexdigit)
        || record[2 * width + 1] != b' '
        || record.contains(&0)
    {
        return Err(bad());
    }
    let oid = object_id(new).map_err(|_| bad())?;
    // committer 名称可以含 TAB；消息也可以含 `> `，所以先定位身份的 `<email>`。
    let ident = &record[2 * width + 2..];
    let opening = ident
        .iter()
        .position(|byte| *byte == b'<')
        .ok_or_else(bad)?;
    let close = ident[opening + 1..]
        .iter()
        .position(|byte| *byte == b'>')
        .map(|at| opening + 1 + at)
        .ok_or_else(bad)?;
    if opening == 0
        || ident[opening - 1] != b' '
        || ident[opening + 1..close].contains(&b'<')
        || ident.get(close + 1) != Some(&b' ')
    {
        return Err(bad());
    }
    let clock = &ident[close + 2..];
    let digits = clock
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if digits == 0 || clock.get(digits) != Some(&b' ') {
        return Err(bad());
    }
    let zone = clock.get(digits + 1..digits + 6).ok_or_else(bad)?;
    if !matches!(zone[0], b'+' | b'-') || !zone[1..].iter().all(u8::is_ascii_digit) {
        return Err(bad());
    }
    let remaining = &clock[digits + 6..];
    if !remaining.is_empty() && remaining[0] != b'\t' {
        return Err(bad());
    }
    Ok(oid)
}
