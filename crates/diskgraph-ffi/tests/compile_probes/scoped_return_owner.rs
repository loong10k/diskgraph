//! PF-06 外部反例：固定 R 不能包含当前宿主生命周期的 owner。
#![forbid(unsafe_code)]
use diskgraph_ffi::{NativeService, NativeServiceError, NativeServiceOwner};

pub fn escape(database: String) -> Result<NativeServiceOwner<'static>, NativeServiceError> {
    NativeService::with_owner(database, |_service, owner| owner)
}
