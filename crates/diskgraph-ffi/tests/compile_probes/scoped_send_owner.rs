//! PF-06 外部反例：即使 scoped thread 不要求 'static，也不能发送宿主 owner。
#![forbid(unsafe_code)]
use diskgraph_ffi::{NativeService, NativeServiceError};

pub fn escape(database: String) -> Result<(), NativeServiceError> {
    NativeService::with_owner(database, |_service, owner| {
        std::thread::scope(|threads| {
            threads.spawn(move || drop(owner)).join().unwrap();
        });
    })
}
