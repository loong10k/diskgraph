use diskgraph_core::BusinessError;
use diskgraph_engine::{
    EngineError, ScanWorkerHost, ScanWorkerHostConfig, ScanWorkerRuntimeBudget,
};
use std::ffi::OsString;
use std::fs::File;
use std::path::PathBuf;
use std::time::Instant;

/// 本地部署来源的不可变 helper 定位与独立预期材料，不从协议或邻接文件建立信任。
/// 来源：原生 Rust PF-06 显式宿主部署合同；无 Java 对等对象。
#[derive(Debug)]
pub struct ScanWorkerSettings {
    path: PathBuf,
    expected: ScanWorkerHostConfig,
}

impl ScanWorkerSettings {
    /// 统一本地CLI/MCP启动时的宿主准入；来源：原生Rust PF-06固定安装信任合同。
    /// 参数：runtime为原响应/进程额度，deadline及checkpoint沿原启动请求；返回宿主、未配置或原错误。
    /// macOS只读取固定root安装，旧环境不能授予资格；其他平台保留独立部署环境的held File路径。
    /// 本方法不建立数据库、不启动helper，不延长原期限；远程请求不得用此入口选择镜像。
    pub fn host_from_environment(
        runtime: ScanWorkerRuntimeBudget,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<Option<ScanWorkerHost>, EngineError> {
        Self::host_from_lookup(runtime, deadline, checkpoint, |name| std::env::var_os(name))
    }

    /// 参数：lookup为本地环境读取器，其余为原准入额度；返回相同平台宿主语义。
    /// 仅crate内测试替换环境读取，不提供安装验签、元数据或固定配置读取的替身。
    pub(crate) fn host_from_lookup(
        runtime: ScanWorkerRuntimeBudget,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
        lookup: impl FnMut(&str) -> Option<OsString>,
    ) -> Result<Option<ScanWorkerHost>, EngineError> {
        check(deadline, checkpoint)?;
        let parsed = Self::from_lookup(lookup);
        // 解析后仍沿用原取消/时钟，包括全部缺失和格式错误；不在中间续期。
        check(deadline, checkpoint)?;
        let configured = parsed?;
        #[cfg(target_os = "macos")]
        let result = {
            if configured.is_some() {
                // 完整旧配置也不能打开普通路径；部分/非法配置已由原解析器拒绝。
                return Err(BusinessError::Unsupported.into());
            }
            ScanWorkerHost::from_installed_macos(runtime, deadline, checkpoint)
        };
        #[cfg(not(target_os = "macos"))]
        let result = match configured {
            Some(settings) => settings.open_held().and_then(|(image, expected)| {
                ScanWorkerHost::new(image, expected, runtime).map(Some)
            }),
            None => Ok(None),
        };
        check(deadline, checkpoint)?;
        result
    }

    /// 在本地服务启动阶段读取三个部署环境值。
    /// 参数：无；返回：完整配置、完全未配置或原配置错误；远程请求不能调用此入口补全材料。
    pub fn from_environment() -> Result<Option<Self>, EngineError> {
        Self::from_lookup(|name| std::env::var_os(name))
    }

    /// 解析受信部署来源的三个环境值；调用方须保证来源属于本地宿主配置。
    /// 参数：lookup 返回原生路径及独立可信摘要/准确长度；返回：不可变材料或明确拒绝。
    /// 完全缺失与部分配置区分，不探测邻接清单，不依赖 PATH 或当前工作目录。
    pub fn from_lookup(
        mut lookup: impl FnMut(&str) -> Option<OsString>,
    ) -> Result<Option<Self>, EngineError> {
        let path = lookup("DISKGRAPH_SCAN_WORKER_PATH");
        let digest = lookup("DISKGRAPH_SCAN_WORKER_SHA256");
        let length = lookup("DISKGRAPH_SCAN_WORKER_BYTES");
        let (path, digest, length) = match (path, digest, length) {
            (None, None, None) => return Ok(None),
            (Some(path), Some(digest), Some(length)) => (PathBuf::from(path), digest, length),
            _ => return Err(BusinessError::InvalidArgument.into()),
        };
        if !path.is_absolute() {
            return Err(BusinessError::InvalidArgument.into());
        }
        let digest = digest.to_str().ok_or(BusinessError::InvalidArgument)?;
        if digest.len() != 64 {
            return Err(BusinessError::InvalidArgument.into());
        }
        let mut expected_sha256 = [0_u8; 32];
        for (target, pair) in expected_sha256
            .iter_mut()
            .zip(digest.as_bytes().as_chunks::<2>().0)
        {
            *target = hex_digit(pair[0])? * 16 + hex_digit(pair[1])?;
        }
        let length = length.to_str().ok_or(BusinessError::InvalidArgument)?;
        if length.is_empty()
            || length.len() > 20
            || !length.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(BusinessError::InvalidArgument.into());
        }
        let expected_bytes = length.parse().map_err(|_| BusinessError::InvalidArgument)?;
        let expected = ScanWorkerHostConfig::from_expected_image(expected_sha256, expected_bytes)?;
        Ok(Some(Self { path, expected }))
    }

    /// 消费部署配置，打开一次原镜像；后续核验/启动只能沿用该 File。
    /// 参数：self 为完整受信配置；返回：唯一已打开 File 与独立预期值，保留实际 I/O 错误。
    /// 本方法不核验摘要、不启动进程、不授予扫描或发布资格。
    pub fn open_held(self) -> Result<(File, ScanWorkerHostConfig), EngineError> {
        Ok((File::open(self.path)?, self.expected))
    }
}

fn check(
    deadline: Instant,
    checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    checkpoint()?;
    if Instant::now() >= deadline {
        return Err(BusinessError::BudgetExceeded.into());
    }
    Ok(())
}

fn hex_digit(byte: u8) -> Result<u8, EngineError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(BusinessError::InvalidArgument.into()),
    }
}
