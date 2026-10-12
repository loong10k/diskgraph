use super::{BrokerQuery, broker_query_request::BrokerQueryRequest};
use std::io;
use std::time::Instant;

/// MCP原连接的有界启动帧；来源：PF-06，无Java对等对象。长度边界避免以stdin EOF结束启动。
pub(super) struct McpSessionRequest;

impl McpSessionRequest {
    pub(super) const PREFIX: &'static [u8] = b"DG_FRONT_MCP_V1\n";

    /// 参数：原启动期限；返回：固定MCP请求，不含客户镜像、环境或授权材料。
    pub(super) fn encode(deadline: Instant) -> io::Result<Vec<u8>> {
        let query = BrokerQuery::new("mcp", None, vec![], vec![])?;
        let body = BrokerQueryRequest::encode(&query, deadline)?;
        let mut bytes = Self::PREFIX.to_vec();
        bytes.extend_from_slice(&(body.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&body);
        if bytes.len() > 4096 {
            return Err(io::Error::other("oversized MCP startup"));
        }
        Ok(bytes)
    }

    /// 参数：累计原字节和原接收期限；返回：完整固定启动、尚缺字节或协议拒绝。
    /// 不接受同一启动读取内的RPC尾随数据，客户必须等待原root握手。
    pub(super) fn decode(
        bytes: &[u8],
        deadline: Instant,
    ) -> io::Result<Option<(BrokerQuery, Instant)>> {
        if bytes.len() > 4096 || !bytes.starts_with(Self::PREFIX) {
            return Err(io::Error::other("invalid MCP startup"));
        }
        let header = Self::PREFIX.len() + 4;
        if bytes.len() < header {
            return Ok(None);
        }
        let length = u32::from_be_bytes(
            bytes[Self::PREFIX.len()..header]
                .try_into()
                .expect("fixed length"),
        ) as usize;
        if length == 0 || length > 4096 - header || bytes.len() > header + length {
            return Err(io::Error::other("invalid MCP startup length or tail"));
        }
        if bytes.len() < header + length {
            return Ok(None);
        }
        let (query, original) = BrokerQueryRequest::decode(&bytes[header..], deadline)?;
        if !query.is_mcp() {
            return Err(io::Error::other("MCP startup command changed"));
        }
        Ok(Some((query, original)))
    }
}
