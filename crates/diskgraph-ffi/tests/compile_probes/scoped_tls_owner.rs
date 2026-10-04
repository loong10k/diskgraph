//! PF-06 外部反例：safe Rust 不能将借用 owner 写入静态 TLS。
#![forbid(unsafe_code)]
use diskgraph_ffi::{NativeService, NativeServiceError, NativeServiceOwner};
use std::cell::RefCell;

thread_local! {
    static OWNER: RefCell<Option<NativeServiceOwner<'static>>> = const { RefCell::new(None) };
}

pub fn escape(database: String) -> Result<(), NativeServiceError> {
    NativeService::with_owner(database, |_service, owner| {
        OWNER.with(|slot| *slot.borrow_mut() = Some(owner));
    })
}
