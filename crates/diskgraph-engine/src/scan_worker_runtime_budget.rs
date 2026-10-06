use crate::EngineError;
use diskgraph_core::BusinessError;
use diskgraph_scan_worker::{ProtocolLimits, WorkerLimits};

/// 宿主显式配置的helper响应额度和有限物理进程容量，不借用staging字节预算。
/// 来源：原生 Rust PF-06 显式宿主合同，无Java对等对象。
#[derive(Clone, Copy, Debug)]
pub struct ScanWorkerRuntimeBudget {
    response: ProtocolLimits,
    stderr_bytes: u64,
    max_active_children: u32,
}

impl ScanWorkerRuntimeBudget {
    /// 参数：response为原单流四项额度，stderr_bytes为总流内子限额，max_active_children为非零槽数。
    /// 返回：原值配置或InvalidArgument；深度0及stderr0不被扩大，不建立执行授权。
    pub fn new(
        response: ProtocolLimits,
        stderr_bytes: u64,
        max_active_children: u32,
    ) -> Result<Self, EngineError> {
        WorkerLimits::from_protocol(response).map_err(|_| BusinessError::InvalidArgument)?;
        if max_active_children == 0 || stderr_bytes > response.max_stream_bytes {
            return Err(BusinessError::InvalidArgument.into());
        }
        Ok(Self {
            response,
            stderr_bytes,
            max_active_children,
        })
    }

    /// 参数：无；返回：宿主原配置的响应额度，不重建或消费执行账本。
    pub fn response_limits(&self) -> ProtocolLimits {
        self.response
    }
    /// 参数：无；返回：原响应总额内部stderr子限额。
    pub fn stderr_bytes(&self) -> u64 {
        self.stderr_bytes
    }
    /// 参数：无；返回：未完成真实wait时不可释放的原容量。
    pub fn max_active_children(&self) -> u32 {
        self.max_active_children
    }
}
