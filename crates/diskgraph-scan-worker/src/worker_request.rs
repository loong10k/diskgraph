use crate::{
    ProtocolLimits, ScanOptions, worker_limits::WorkerLimits, worker_path::WorkerPath,
    worker_scan_request::WorkerScanRequest, worker_terminal_budget::WorkerTerminalBudget,
};
use diskgraph_disktree_core::scan;
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};

/// 单次 helper 执行输入与后续取消命令；来源：PF-06 v2，不改变旧 Frame::Request 的形状。
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkerRequest {
    Request {
        version: u32,
        request: WorkerScanRequest,
        limits: WorkerLimits,
    },
    Cancel {},
}

impl WorkerRequest {
    /// 构造唯一执行 v2 请求；只携带事实输入，不携带数据库、主体、token 或 fence。
    /// 参数：root 为真实绝对根，options 为 pinned 的全部七项选项，limits 为原传输额度。
    /// 返回：当前平台路径和额度已校验的请求；原额度须承载 Hello 与最大固定预算终态。
    /// 仅做执行请求准入，不提供 OS 执行或发布许可。
    pub fn scan(
        root: &Path,
        options: &scan::ScanOptions,
        limits: ProtocolLimits,
    ) -> io::Result<Self> {
        let admitted = WorkerLimits::from_protocol(limits)?;
        WorkerTerminalBudget::check(limits)?;
        Ok(Self::Request {
            version: 2,
            request: WorkerScanRequest {
                root: WorkerPath::from_path(root)?,
                options: ScanOptions::from_native(options),
            },
            limits: admitted,
        })
    }

    /// 按 helper 原准备顺序恢复 v2 扫描输入，外部构造的公开 variant 也必须再次校验。
    /// 参数：self 为完整闭合解码或父端构造的唯一请求。
    /// 返回：无损 root、原 pinned 选项和原额度；取消、非 v2 或无法承载握手及预算终态的请求拒绝。
    pub fn into_scan(self) -> io::Result<(PathBuf, scan::ScanOptions, ProtocolLimits)> {
        match self {
            Self::Request {
                version: 2,
                request,
                limits,
            } => {
                let limits = limits.checked()?;
                WorkerTerminalBudget::check(limits)?;
                let options = request.options.to_native()?;
                let root = request.root.into_path()?;
                Ok((root, options, limits))
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "expected one execution Request version 2",
            )),
        }
    }
}
