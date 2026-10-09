//! 配置只作为数据解析；生成的配置不含仓库外部程序能力。

use std::collections::BTreeSet;

/// 保存有序配置字段并生成受限执行配置。
/// 来源：原生 Rust DiskGraph Git 私有视图与 Git NUL 配置协议，无 Java 对应实现。
#[derive(Default)]
pub(super) struct GitConfiguration {
    entries: Vec<(String, Vec<u8>)>,
}

impl GitConfiguration {
    /// 合并已捕获配置的原生解析结果，使用原期限与取消状态。
    /// 参数：captured 为原始完整字节，probe 为原预算，parse 为非空配置解析操作。
    /// 返回：保留顺序的原生输出或原失败；不捕获源文件或重置任何额度。
    pub(super) fn parse_captured(
        &mut self,
        captured: &[u8],
        probe: &mut super::probe_budget::ProbeBudget,
        parse: impl FnOnce(&mut super::probe_budget::ProbeBudget) -> Result<Vec<u8>, String>,
    ) -> Result<Vec<u8>, String> {
        probe.check().map_err(|error| error.to_string())?;
        // 只有完整的零字节捕获可确定没有字段；空白及注释仍交给原生 Git。
        // 源文件仍保留在元数据集合中，由原来的末段复核检查身份及内容变化。
        if captured.is_empty() {
            return Ok(Vec::new());
        }
        let parsed = parse(probe)?;
        probe.check().map_err(|error| error.to_string())?;
        self.extend(&parsed)?;
        Ok(parsed)
    }

    /// 判断捕获的配置是否声明 filter driver。
    /// 参数：无。返回：存在 driver 字段时 true；不能由此判断工作树是否应用 driver。
    pub(super) fn has_filters(&self) -> bool {
        self.entries
            .iter()
            .any(|(key, _)| key.starts_with("filter."))
    }
    /// 合并 Git --null --list 的字段，保留多值及覆盖顺序。
    /// 参数：bytes 为从私有配置副本解析的完整输出。
    /// 返回：字段加入成功，或格式/不支持的配置错误。
    pub(super) fn extend(&mut self, bytes: &[u8]) -> Result<(), String> {
        if !bytes.is_empty() && !bytes.ends_with(&[0]) {
            return Err("invalid unterminated Git configuration".into());
        }
        for record in bytes.split_inclusive(|byte| *byte == 0) {
            let record = &record[..record.len() - 1];
            let split = record.iter().position(|byte| *byte == b'\n');
            let (key, value) = split.map_or((record, b"true".as_slice()), |at| {
                (&record[..at], &record[at + 1..])
            });
            let key = std::str::from_utf8(key).map_err(|_| "unsupported Git configuration key")?;
            if key.is_empty() || !key.contains('.') || key.contains(['\n', '\r', '\0']) {
                return Err("invalid Git configuration key".into());
            }
            let lower = key.to_ascii_lowercase();
            if lower.starts_with("include.") || lower.starts_with("includeif.") {
                return Err("unsupported Git configuration include".into());
            }
            if lower.starts_with("extensions.")
                && !matches!(
                    lower.as_str(),
                    "extensions.objectformat" | "extensions.worktreeconfig"
                )
            {
                return Err(format!("unsupported Git repository extension: {key}"));
            }
            if lower.starts_with("remote.")
                && (lower.ends_with(".promisor") || lower.ends_with(".partialclonefilter"))
            {
                return Err("unsupported Git partial/promisor repository".into());
            }
            self.entries.push((key.to_owned(), value.to_vec()));
        }
        Ok(())
    }

    /// 查询单值字段的最终覆盖值，不改变 subsection 的大小写。
    /// 参数：key 为普通不含 subsection 的字段名。
    /// 返回：最后一个匹配值或没有设置。
    pub(super) fn last(&self, key: &str) -> Option<&[u8]> {
        self.entries
            .iter()
            .rev()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.as_slice())
    }

    /// 确认普通 files 后端与原生对象格式。
    /// 参数：无；读取捕获的有效配置。
    /// 返回：OID 字节数 20/32；特殊格式和会改变 index 解释的配置明确拒绝。
    pub(super) fn oid_len(&self) -> Result<usize, String> {
        if self
            .last("extensions.worktreeconfig")
            .is_some_and(|value| super::git_config_policy::boolean(value).is_none())
        {
            return Err("unsupported Git worktree configuration value".into());
        }
        for key in [
            "core.bare",
            "core.sparsecheckout",
            "core.sparsecheckoutcone",
            "core.splitindex",
        ] {
            if self
                .last(key)
                .is_some_and(|value| super::git_config_policy::boolean(value) != Some(false))
            {
                return Err(format!("unsupported Git configuration: {key}"));
            }
        }
        if self
            .last("core.repositoryformatversion")
            .is_some_and(|value| !matches!(value, b"0" | b"1"))
        {
            return Err("unsupported Git repository format version".into());
        }
        match self.last("extensions.objectformat") {
            None | Some(b"sha1") => Ok(20),
            Some(b"sha256") => Ok(32),
            _ => Err("unsupported Git object format".into()),
        }
    }

    /// 生成保留状态语义的固定配置；driver 无命令且 required，应用时失败。
    /// 参数：global_attributes/global_excludes 为已捕获数据的私有绝对路径字节。
    /// 返回：配置字节，或不能保真的字段错误；外部程序/URL/helper 永不复制。
    pub(super) fn render(
        &self,
        global_attributes: &[u8],
        global_excludes: &[u8],
    ) -> Result<Vec<u8>, String> {
        self.oid_len()?;
        let mut out = Vec::new();
        let mut filters = BTreeSet::new();
        for (key, value) in &self.entries {
            let lower = key.to_ascii_lowercase();
            if let Some(driver) = key
                .strip_prefix("filter.")
                .and_then(|rest| rest.rsplit_once('.').map(|parts| parts.0))
            {
                filters.insert(driver.to_owned());
            } else if super::git_config_policy::preserved(&lower, value)? {
                write_field(&mut out, key, value)?;
            }
        }
        for driver in filters {
            write_field(&mut out, &format!("filter.{driver}.required"), b"true")?;
        }
        for (key, value) in [
            ("core.fsmonitor", b"false".as_slice()),
            ("core.untrackedcache", b"false".as_slice()),
            ("core.attributesfile", global_attributes),
            ("core.excludesfile", global_excludes),
        ] {
            write_field(&mut out, key, value)?;
        }
        Ok(out)
    }
}

fn write_field(out: &mut Vec<u8>, key: &str, value: &[u8]) -> Result<(), String> {
    let (section, rest) = key.split_once('.').ok_or("invalid Git configuration key")?;
    let (subsection, name) = rest
        .rsplit_once('.')
        .map_or((None, rest), |(sub, name)| (Some(sub), name));
    if !section
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || section.is_empty()
        || name.is_empty()
        || value.contains(&0)
    {
        return Err("unsupported Git configuration field".into());
    }
    out.push(b'[');
    out.extend_from_slice(section.as_bytes());
    if let Some(subsection) = subsection {
        out.extend_from_slice(b" \"");
        escape(out, subsection.as_bytes());
        out.push(b'"');
    }
    out.extend_from_slice(b"]\n\t");
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(b" = \"");
    escape(out, value);
    out.extend_from_slice(b"\"\n");
    Ok(())
}

fn escape(out: &mut Vec<u8>, value: &[u8]) {
    for byte in value {
        match byte {
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'"' => out.extend_from_slice(b"\\\""),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\t' => out.extend_from_slice(b"\\t"),
            _ => out.push(*byte),
        }
    }
}
