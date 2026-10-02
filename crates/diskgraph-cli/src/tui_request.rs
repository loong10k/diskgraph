use diskgraph_core::{Authorizer, PrincipalId};
use diskgraph_engine::Engine;

/// 终端会话的不可变请求身份；每帧和导航读取仍检查实时授权。
pub struct TuiRequest<'a> {
    pub engine: &'a Engine,
    pub revision: &'a str,
    pub principal: &'a PrincipalId,
    pub authorizer: &'a dyn Authorizer,
}
