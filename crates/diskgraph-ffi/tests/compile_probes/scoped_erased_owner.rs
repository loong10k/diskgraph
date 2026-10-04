//! PF-06 外部反例：'static 闭包类型擦除不能隐藏借用 owner 的生命周期。
#![forbid(unsafe_code)]
use diskgraph_ffi::{NativeService, NativeServiceError};

pub fn escape(database: String) -> Result<Box<dyn FnOnce() + 'static>, NativeServiceError> {
    NativeService::with_owner(database, |_service, owner| {
        Box::new(move || drop(owner)) as Box<dyn FnOnce() + 'static>
    })
}
