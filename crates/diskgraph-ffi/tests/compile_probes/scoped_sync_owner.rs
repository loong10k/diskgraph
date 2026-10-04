//! PF-06 外部反例：借用能力也不能通过共享引用传入另一线程。
#![forbid(unsafe_code)]
use diskgraph_ffi::{NativeService, NativeServiceError};

pub fn escape(database: String) -> Result<(), NativeServiceError> {
    NativeService::with_owner(database, |_service, owner| {
        std::thread::scope(|threads| {
            let borrowed = &owner;
            threads
                .spawn(move || std::hint::black_box(borrowed))
                .join()
                .unwrap();
        });
    })
}
