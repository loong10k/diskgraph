use super::{ControlError, ControlNotification};
use serde::{Deserialize, Serialize};
/// 固定协议版本、原会话绑定和单调序列的控制帧；来源：PF-06，无 Java 对等对象。
/// session 必须来自已认证私有通道，随机标识本身不建立传输信任。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlFrame {
    pub version: u16,
    pub session: [u8; 32],
    pub sequence: u64,
    pub notification: ControlNotification,
}
impl ControlFrame {
    /// 参数：self 为监督通知；返回：长度前缀与有界 JSON，不写入公共 stdout/MCP wire。
    pub fn encode(&self) -> Result<Vec<u8>, ControlError> {
        let body = serde_json::to_vec(self).map_err(|_| ControlError::Protocol)?;
        if body.len() > 4096 {
            return Err(ControlError::Budget);
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(body.len() + 4)
            .map_err(|_| ControlError::Budget)?;
        bytes.extend_from_slice(&(body.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&body);
        Ok(bytes)
    }
}
