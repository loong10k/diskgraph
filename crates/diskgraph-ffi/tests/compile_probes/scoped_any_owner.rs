//! PF-06 外部反例：Any 的静态要求不能擦除 owner brand。
#![forbid(unsafe_code)]
use diskgraph_ffi::{NativeService, NativeServiceError};
use std::any::Any;

pub fn escape(database: String) -> Result<Box<dyn Any>, NativeServiceError> {
    NativeService::with_owner(database, |_service, owner| Box::new(owner) as Box<dyn Any>)
}
