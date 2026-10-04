//! PF-06 外部消费者正控：固定返回值或 Service 可以逃逸，owner 能力留在宿主作用域。
#![forbid(unsafe_code)]
use diskgraph_ffi::{NativeService, NativeServiceError};
use std::sync::Arc;

pub fn complete(database: String) -> Result<(), NativeServiceError> {
    NativeService::with_owner(database, |service, mut owner| {
        drop(service);
        owner.finalize_owner()?;
        owner.finalize_owner()
    })?
}

pub fn closed_service(database: String) -> Result<Arc<NativeService>, NativeServiceError> {
    NativeService::with_owner(database, |service, owner| {
        std::mem::forget(owner);
        service
    })
}

/// 独立嵌套作用域允许各自 finalize/drop，不交换 brand。
pub fn nested(first: String, second: String) -> Result<(), NativeServiceError> {
    NativeService::with_owner(first, |_first_service, outer| {
        NativeService::with_owner(second, |_second_service, inner| drop(inner)).unwrap();
        drop(outer);
    })
}
