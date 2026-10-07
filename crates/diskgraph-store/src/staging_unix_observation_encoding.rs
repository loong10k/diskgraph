//! Unix暂存旁表的共享编码与逻辑字节计量。
use crate::{Result, StoreError};
use diskgraph_core::{UnixFileObservation, UnixObservationGap};

/// 实际旁表载荷及编码成本；来源：DiskGraph原生Rust，非Java迁移。
pub(crate) struct StagingUnixObservationEncoding {
    pub(crate) raw: Option<Vec<u8>>,
    pub(crate) gap: Option<&'static str>,
    pub(crate) cost: u64,
}
impl StagingUnixObservationEncoding {
    /// 参数：互斥完整观察或gap；返回：沿原校验的实际载荷和checked逻辑成本。
    pub(crate) fn encode(
        observation: Option<&UnixFileObservation>,
        gap: Option<UnixObservationGap>,
    ) -> Result<Self> {
        if observation.is_some() == gap.is_some() {
            return Err(StoreError::InvalidGraph(
                "invalid Unix observation payload".into(),
            ));
        }
        let raw = observation
            .map(|value| {
                value
                    .encode()
                    .map_err(|error| StoreError::InvalidGraph(error.into()))
            })
            .transpose()?;
        let gap = gap.map(|value| value.code());
        // 计入实际载荷以及两个固定宽度整数；SQLite物理页占用由容量门禁另行检查。
        let payload_len = raw.as_ref().map_or_else(|| gap.unwrap().len(), Vec::len);
        let cost = u64::try_from(payload_len)
            .map_err(|_| StoreError::IntegerOverflow)?
            .checked_add(16)
            .ok_or(StoreError::IntegerOverflow)?;
        Ok(Self { raw, gap, cost })
    }
}
/// 计算Unix旁表的实际编码预算；参数：恰有一个观察或gap；返回：checked逻辑字段字节数。
/// 计量与Store写入共用校验和编码，不估计SQLite页/索引及公共namespace键。
pub fn staging_unix_observation_encoded_cost(
    observation: Option<&UnixFileObservation>,
    gap: Option<UnixObservationGap>,
) -> Result<u64> {
    Ok(StagingUnixObservationEncoding::encode(observation, gap)?.cost)
}
