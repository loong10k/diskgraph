//! Git 来源发现和 scoped 映射；来源：原生 Rust GitView，保持单一视图 owner。
use super::git_native_path;
use super::git_view::GitView;
use super::probe_budget::ProbeBudget;
use std::path::{Path, PathBuf};

impl GitView {
    /// 参数：configuration 为已解析设置，worktree 为私有项目，probe 为原预算；返回：所有显式属性依赖先捕获再解析，私有名单保持稳定。
    pub(super) fn capture_configured_dependencies(
        &mut self,
        configuration: &super::git_configuration::GitConfiguration,
        worktree: &Path,
        probe: &mut ProbeBudget,
    ) -> Result<(), String> {
        if self.source_capture.is_some() {
            for key in ["core.attributesfile", "core.excludesfile"] {
                if let Some(value) = configuration.last(key).filter(|value| !value.is_empty()) {
                    if value.starts_with(b"~") || value.starts_with(b"%(prefix)") {
                        return Err("unsupported Git configuration path interpolation".into());
                    }
                    self.resolve_source(worktree, &git_native_path::from_bytes(value)?, probe)?;
                }
            }
        }
        Ok(())
    }

    /// 参数：base/value 为原配置路径，probe 为共享预算；返回：范围内依赖的私有映射，可信旧入口保持原路径。
    pub(super) fn resolve_source(
        &mut self,
        base: &Path,
        value: &Path,
        probe: &mut ProbeBudget,
    ) -> Result<PathBuf, String> {
        match &mut self.source_capture {
            Some(capture) => capture.resolve(
                base,
                value,
                &mut self.directory,
                &mut self.metadata_budget,
                probe,
            ),
            None => git_native_path::resolve(base, value),
        }
    }

    /// 参数：project 为当前工作树路径，probe 为共享预算；返回：工作树与元数据路径，scoped 不发现祖先。
    pub(super) fn locate(
        &mut self,
        project: &Path,
        probe: &mut ProbeBudget,
    ) -> Result<(PathBuf, PathBuf), String> {
        let mut root = project.to_owned();
        loop {
            self.metadata_budget.check(probe)?;
            let path = root.join(".git");
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                    return Ok((root, path));
                }
                Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                    let index =
                        self.metadata
                            .detached_file(&path, &mut self.metadata_budget, probe)?;
                    let bytes =
                        trim_line(self.metadata.get(index).bytes().expect("captured gitfile"));
                    let directory = bytes
                        .strip_prefix(b"gitdir: ")
                        .ok_or("invalid Git gitfile")?;
                    return Ok((
                        root.clone(),
                        self.resolve_source(
                            &root,
                            &git_native_path::from_bytes(directory)?,
                            probe,
                        )?,
                    ));
                }
                Ok(_) => return Err("unsupported Git gitfile type".into()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("Git discovery: {error}")),
            }
            if self.source_capture.is_some() || !root.pop() {
                return Err("Git repository was not found".into());
            }
        }
    }
}

/// 参数：bytes 为 Git 行记录；返回：只去除单个 CRLF/LF 的原字节借用。
pub(super) fn trim_line(bytes: &[u8]) -> &[u8] {
    bytes
        .strip_suffix(b"\n")
        .unwrap_or(bytes)
        .strip_suffix(b"\r")
        .unwrap_or(bytes.strip_suffix(b"\n").unwrap_or(bytes))
}

/// 参数：bytes 为 packed-refs 内容；返回：是否含 replacement 引用。
pub(super) fn packed_replace(bytes: &[u8]) -> bool {
    bytes
        .split(|byte| *byte == b'\n')
        .any(|line| line.windows(13).any(|part| part == b"refs/replace/"))
}
