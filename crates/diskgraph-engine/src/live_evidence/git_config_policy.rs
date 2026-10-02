//! 仅允许不会执行程序、且对状态计算有必要的已知字段。

/// 解析 Git 布尔语义；裸字段已由解析器转成 true。
/// 参数：value 为原始配置值。返回：已知布尔值，否则 None。
pub(super) fn boolean(value: &[u8]) -> Option<bool> {
    match value.to_ascii_lowercase().as_slice() {
        b"true" | b"yes" | b"on" | b"1" => Some(true),
        b"false" | b"no" | b"off" | b"0" | b"" => Some(false),
        _ => None,
    }
}

/// 校验配置的数据类型并决定是否保留。
/// 参数：key 为大小写规范化后的字段名，value 为原始值。
/// 返回：已知安全语义字段为 true；无关字段为 false；未知 core 状态语义拒绝。
pub(super) fn preserved(key: &str, value: &[u8]) -> Result<bool, String> {
    let bool_key = matches!(
        key,
        "core.filemode"
            | "core.ignorecase"
            | "core.symlinks"
            | "core.trustctime"
            | "core.ignorestat"
            | "core.precomposeunicode"
            | "diff.indentheuristic"
    );
    if bool_key {
        return boolean(value)
            .map(|_| true)
            .ok_or_else(|| format!("unsupported Git boolean: {key}"));
    }
    let allowed = match key {
        "core.autocrlf" => boolean(value).is_some() || value == b"input",
        "core.eol" => matches!(value, b"lf" | b"crlf" | b"native"),
        "core.safecrlf" => boolean(value).is_some() || value == b"warn",
        "core.checkstat" => matches!(value, b"default" | b"minimal"),
        "core.repositoryformatversion" => matches!(value, b"0" | b"1"),
        "extensions.objectformat" => matches!(value, b"sha1" | b"sha256"),
        "core.bigfilethreshold" | "diff.renamelimit" | "status.renamelimit" => integer(value),
        "status.renames" | "diff.renames" => {
            boolean(value).is_some() || matches!(value, b"copies" | b"copy")
        }
        "diff.algorithm" => matches!(
            value,
            b"myers" | b"minimal" | b"patience" | b"histogram" | b"default"
        ),
        "core.checkroundtripencoding" => !value.contains(&0),
        _ => return other(key, value),
    };
    if allowed {
        Ok(true)
    } else {
        Err(format!("unsupported Git value: {key}"))
    }
}

fn integer(value: &[u8]) -> bool {
    let digits = value
        .strip_suffix(b"k")
        .or_else(|| value.strip_suffix(b"m"))
        .or_else(|| value.strip_suffix(b"g"))
        .unwrap_or(value);
    !digits.is_empty() && digits.iter().all(u8::is_ascii_digit) && digits.len() <= 18
}

fn other(key: &str, _value: &[u8]) -> Result<bool, String> {
    if (key.starts_with("branch.") && (key.ends_with(".remote") || key.ends_with(".merge")))
        || (key.starts_with("remote.") && key.ends_with(".fetch"))
    {
        return Ok(true);
    }
    if key.starts_with("core.")
        && !matches!(
            key,
            "core.fsmonitor"
                | "core.fsmonitorhookversion"
                | "core.untrackedcache"
                | "core.attributesfile"
                | "core.excludesfile"
                | "core.bare"
                | "core.sparsecheckout"
                | "core.sparsecheckoutcone"
                | "core.splitindex"
                | "core.worktree"
                | "core.hookspath"
                | "core.pager"
                | "core.editor"
                | "core.sshcommand"
                | "core.askpass"
                | "core.logallrefupdates"
                | "core.sharedrepository"
                | "core.quotepath"
                | "core.abbrev"
                | "core.compression"
                | "core.loosecompression"
                | "core.packedgitlimit"
                | "core.packedgitwindowsize"
                | "core.deltabasecachelimit"
                | "core.preloadindex"
                | "core.fsync"
                | "core.fsyncmethod"
                | "core.fsyncobjectfiles"
                | "core.commitgraph"
                | "core.multipackindex"
                | "core.longpaths"
                | "core.protecthfs"
                | "core.protectntfs"
                | "core.warnambiguousrefs"
        )
    {
        return Err(format!("unsupported Git core semantics: {key}"));
    }
    Ok(false)
}
