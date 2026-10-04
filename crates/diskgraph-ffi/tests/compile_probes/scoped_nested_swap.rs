//! PF-06 外部反例：私有不变 brand 不能在不同宿主作用域间交换。
#![forbid(unsafe_code)]
use diskgraph_ffi::{NativeService, NativeServiceError};

pub fn escape(first: String, second: String) -> Result<(), NativeServiceError> {
    NativeService::with_owner(first, |_first_service, mut outer| {
        NativeService::with_owner(second, |_second_service, mut inner| {
            std::mem::swap(&mut outer, &mut inner);
        })
        .unwrap();
    })
}
