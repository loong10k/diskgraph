use super::BrokerQuery;
use crate::native_deadline::ClockStamp;
use serde::{Deserialize, Serialize};
use std::io;
use std::time::{Duration, Instant};

/// 版本化查询与原客户端时钟的单次有限请求。来源：PF-06，无Java对等对象。
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BrokerQueryRequest {
    deadline: ClockStamp,
    query: BrokerQuery,
}

impl BrokerQueryRequest {
    /// 参数：固定查询和原期限；返回：完整验证且不超过4096字节的版本化正文。
    pub(super) fn encode(query: &BrokerQuery, deadline: Instant) -> io::Result<Vec<u8>> {
        query.validate()?;
        let request = Self {
            deadline: ClockStamp::capture(deadline).map_err(io::Error::other)?,
            query: query.clone(),
        };
        let mut bytes = b"DG_FRONT_QUERY_V1\n".to_vec();
        bytes.extend_from_slice(&serde_json::to_vec(&request)?);
        if bytes.len() > 4096 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "oversized broker query",
            ));
        }
        Ok(bytes)
    }

    /// 参数：原正文与接收开始期限；返回：已验证查询和两份原期限的交集，不刷新业务时间。
    pub(super) fn decode(bytes: &[u8], deadline: Instant) -> io::Result<(BrokerQuery, Instant)> {
        if bytes.len() > 4096 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "oversized broker query",
            ));
        }
        let payload = bytes.strip_prefix(b"DG_FRONT_QUERY_V1\n").ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "unknown broker query version")
        })?;
        let request: Self = serde_json::from_slice(payload)?;
        request.query.validate()?;
        let original = request
            .deadline
            .adopt(Duration::from_secs(30))
            .map_err(io::Error::other)?;
        Ok((request.query, original.min(deadline)))
    }
}
