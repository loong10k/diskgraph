use serde::Deserialize;
use std::io;
use std::path::PathBuf;

/// 固定Linux监督安装配置的数据契约。来源：PF-06可信安装者；无Java对等对象。
/// 解码只验证数据结构，不授予root、镜像执行或目录信任；公开请求不得提供本配置。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallationConfig {
    pub(super) schema_version: u8,
    pub(super) frontend_uid: u32,
    pub(super) service_uid: u32,
    pub(super) service_gid: u32,
    pub(super) state_dir: PathBuf,
    pub(super) data_dir: PathBuf,
    pub(super) socket_path: PathBuf,
    pub(super) cli_path: PathBuf,
    pub(super) cli_sha256: String,
    pub(super) worker_path: PathBuf,
    pub(super) worker_sha256: String,
    #[serde(default)]
    pub(super) mcp_path: Option<PathBuf>,
    #[serde(default)]
    pub(super) mcp_sha256: Option<String>,
}

impl InstallationConfig {
    /// 参数：path为可信安装者固定配置位置，deadline为原期限；返回：原root文件的已核验配置快照。
    /// 逐组件no-follow，最多4097字节观察以拒绝增长；普通用户文件、链接及读取期间修改均拒绝。
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub fn open_root(path: &std::path::Path, deadline: std::time::Instant) -> io::Result<Self> {
        use super::root_installation_path::{check, denied, identity};
        use std::io::Read;
        let (mut original, _directories) = super::root_installation_path::open(path, deadline)?;
        let before = original.metadata()?;
        if before.len() > 4096 {
            return Err(invalid("oversized installation configuration"));
        }
        let mut bytes = [0; 4097];
        let mut used = 0;
        loop {
            check(deadline)?;
            let count = original.read(&mut bytes[used..])?;
            check(deadline)?;
            if count == 0 {
                break;
            }
            used += count;
            if used > 4096 {
                return Err(invalid("oversized installation configuration"));
            }
        }
        let config = Self::decode(&bytes[..used])?;
        if identity(&before) != identity(&original.metadata()?) {
            return Err(denied("installation configuration changed during reading"));
        }
        check(deadline)?;
        Ok(config)
    }

    /// 参数：bytes为安装者预置的有限JSON；返回：解析配置或明确拒绝。
    /// 成功不代表安装来源受信，执行前仍须核验原文件、目录链和完整镜像。
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() > 4096 {
            return Err(invalid("oversized installation configuration"));
        }
        let config: Self = serde_json::from_slice(bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if config.schema_version != 1
            || [config.frontend_uid, config.service_uid, config.service_gid]
                .iter()
                .any(|identity| *identity == 0 || *identity == u32::MAX)
            || config.frontend_uid == config.service_uid
        {
            return Err(invalid("invalid fixed service identity or schema"));
        }
        if config.mcp_path.is_some() != config.mcp_sha256.is_some() {
            return Err(invalid("incomplete fixed MCP image binding"));
        }
        for path in [
            &config.state_dir,
            &config.data_dir,
            &config.socket_path,
            &config.cli_path,
            &config.worker_path,
        ]
        .into_iter()
        .chain(config.mcp_path.iter())
        {
            let value = path
                .to_str()
                .ok_or_else(|| invalid("non-UTF8 installation path"))?;
            // 此协议是Linux安装布局；按POSIX原文本验证，不能采用Windows宿主的绝对路径语义。
            // Path::components还会折叠部分点和重复分隔符，不能把注入规范化成成功。
            if !value.starts_with('/')
                || value.len() > 1024
                || value.contains('\0')
                || value.contains('\\')
                || value[1..]
                    .split('/')
                    .any(|component| component.is_empty() || component == "." || component == "..")
            {
                return Err(invalid("noncanonical fixed installation path"));
            }
        }
        let state = config.state_dir.to_str().expect("validated POSIX path");
        let data = config.data_dir.to_str().expect("validated POSIX path");
        if !data
            .strip_prefix(state)
            .is_some_and(|suffix| suffix.starts_with('/') && suffix.len() > 1)
            || config
                .socket_path
                .to_str()
                .expect("validated POSIX path")
                .len()
                > 100
            || config.cli_path == config.worker_path
            || config
                .mcp_path
                .as_ref()
                .is_some_and(|path| path == &config.cli_path || path == &config.worker_path)
        {
            return Err(invalid("invalid fixed installation layout"));
        }
        for digest in [&config.cli_sha256, &config.worker_sha256]
            .into_iter()
            .chain(config.mcp_sha256.iter())
        {
            if digest.len() != 64
                || !digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err(invalid("invalid complete installation SHA256"));
            }
        }
        Ok(config)
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
