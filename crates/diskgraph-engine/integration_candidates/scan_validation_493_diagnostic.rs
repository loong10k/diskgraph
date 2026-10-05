//! 固定 493 隔离构建的 namespace 首错观察；原生产检查、flags 和错误优先级不变。
use crate::EngineError;
use diskgraph_core::{BusinessError, ProcessEvidenceFailureCode as Failure};
use std::cell::Cell;
use std::io::{Cursor, Write};
use std::time::Instant;

thread_local! {
    // 原执行时钟、验证次数及原请求比较结果；不保存身份字符串。
    static CONTEXT: Cell<Option<(Instant, [u64; 3])>> = const { Cell::new(None) };
    // operation、errno、组件序号、dev/ino/mount 布尔比较；只保存当前同步验证。
    static SLOT: Cell<Option<[i64; 6]>> = const { Cell::new(None) };
    #[cfg(test)]
    static EMITTED: Cell<Option<[i64; 8]>> = const { Cell::new(None) };
}

/// 一次执行或一次 namespace validate 的有界观察对象；来源：DiskGraph 原生 Rust 493 诊断。
/// TLS 仅定位本线程同步调用栈，既不缓存授权，也不改变原返回优先级。
pub(crate) struct ScanValidation493Diagnostic {
    execution: bool,
    previous_context: Option<(Instant, [u64; 3])>,
    previous_slot: Option<[i64; 6]>,
    observed_context: Option<(Instant, [u64; 3])>,
    native_failure: Cell<Option<Failure>>,
    returned_category: Cell<u8>,
}

impl ScanValidation493Diagnostic {
    /// 参数：started 是原 scan_started；两个布尔值来自真实 job 与本次 benchmark 请求比较。
    /// 返回：执行局部租约，退出或 unwind 后恢复上一层；不输出具体请求身份。
    pub(crate) fn execution(
        started: Instant,
        owner_matches: bool,
        principal_matches: bool,
    ) -> Self {
        let previous_context = CONTEXT.with(|slot| {
            slot.replace(Some((
                started,
                [0, u64::from(owner_matches), u64::from(principal_matches)],
            )))
        });
        let previous_slot = SLOT.with(|slot| slot.replace(None));
        Self {
            execution: true,
            previous_context,
            previous_slot,
            observed_context: None,
            native_failure: Cell::new(None),
            returned_category: Cell::new(0),
        }
    }

    /// 参数：无；返回：当前原生验证的空首错槽，上一验证 errno 不得污染本次。
    pub(super) fn begin() -> Self {
        let observed_context = CONTEXT.with(|slot| {
            let mut current = slot.get();
            if let Some((_, counters)) = &mut current {
                counters[0] = counters[0].saturating_add(1);
            }
            slot.set(current);
            current
        });
        let previous_slot = SLOT.with(|slot| slot.replace(Some([0, i64::MIN, -1, -1, -1, -1])));
        Self {
            execution: false,
            previous_context: None,
            previous_slot,
            observed_context,
            native_failure: Cell::new(None),
            returned_category: Cell::new(0),
        }
    }

    /// 参数：index 是锚零或祖先组件序号；返回：无，首错出现后不再改变位置。
    pub(super) fn component(index: usize) {
        SLOT.with(|slot| {
            if let Some(mut record) = slot.get().filter(|value| value[0] == 0) {
                record[2] = i64::try_from(index).unwrap_or(i64::MAX);
                slot.set(Some(record));
            }
        });
    }

    /// 参数：operation 固定为 1=openat2、2=statx、3=fstat；errno 在原失败处立即捕获。
    /// 返回：无，仅保存当前验证第一次原生失败，不读取后来的线程 errno。
    pub(super) fn native(operation: u8, errno: Option<i32>) {
        SLOT.with(|slot| {
            if let Some(mut record) = slot.get().filter(|value| value[0] == 0) {
                record[0] = i64::from(operation);
                record[1] = errno.map_or(i64::MIN, i64::from);
                slot.set(Some(record));
            }
        });
    }

    /// 参数：三个布尔值来自原 held 对象与当前实际对象比较；返回：无，不保存具体 ID。
    pub(super) fn identity(device: bool, inode: bool, mount: bool) {
        SLOT.with(|slot| {
            if let Some(mut record) = slot.get().filter(|value| value[0] == 0) {
                record[0] = 4;
                record[3] = i64::from(device);
                record[4] = i64::from(inode);
                record[5] = i64::from(mount);
                slot.set(Some(record));
            }
        });
    }

    /// 参数：result 是 namespace.verify 在 checked_native 闭包内的原结果，尚未 post-check。
    /// 返回：无；保留首个原生失败，即使后续原检查先返回或 unwind 也不丢失。
    pub(super) fn observe_native(&self, result: &Result<(), Failure>) {
        if self.native_failure.get().is_none()
            && let Err(failure) = result
        {
            self.native_failure.set(Some(*failure));
        }
    }

    /// 参数：result 是按原检查及 mapper 得到的最终 Engine 结果；返回：无，只借用闭合类别。
    /// 不拥有、克隆或格式化原错误；清理包装仍按其原主错误分类。
    pub(super) fn returned(&self, result: &Result<(), EngineError>) {
        self.returned_category
            .set(match result.as_ref().err().map(EngineError::primary) {
                Some(EngineError::Business(BusinessError::Conflict)) => 1,
                Some(EngineError::Business(BusinessError::BudgetExceeded)) => 2,
                Some(EngineError::Business(BusinessError::PermissionDenied)) => 3,
                Some(EngineError::Business(_)) => 4,
                Some(EngineError::Store(_)) => 5,
                Some(EngineError::Io(_)) => 6,
                Some(EngineError::Poisoned) => 7,
                Some(EngineError::WithCleanup { .. }) | None => 0,
            });
    }

    fn encode(
        &self,
        failure: Failure,
        record: [i64; 6],
        returned: u8,
        output: &mut dyn Write,
    ) -> std::io::Result<()> {
        let Some((started, counters)) = self.observed_context else {
            return Ok(());
        };
        let operation = match record[0] {
            1 => "openat2",
            2 => "statx",
            3 => "fstat",
            4 => "identity",
            _ => "unknown",
        };
        let category = match returned {
            1 => "business_conflict",
            2 => "business_budget_exceeded",
            3 => "business_permission_denied",
            4 => "business_other",
            5 => "store",
            6 => "io",
            7 => "poisoned",
            8 => "unwind",
            _ => "unknown",
        };
        write!(
            output,
            "DG_SCAN_VALIDATION_493_FIRST_FAILURE {{\"phase\":\"namespace_validate\",\"operation\":\"{operation}\",\"native_failure\":\"{}\",\"errno\":",
            failure.as_str()
        )?;
        if record[1] == i64::MIN {
            write!(output, "null")?;
        } else {
            write!(output, "{}", record[1])?;
        }
        write!(output, ",\"component\":")?;
        if record[2] < 0 {
            write!(output, "null")?;
        } else {
            write!(output, "{}", record[2])?;
        }
        write!(
            output,
            ",\"elapsed_us\":{},\"validation\":{},\"owner_matches\":{},\"principal_matches\":{}",
            started.elapsed().as_micros(),
            counters[0],
            counters[1] != 0,
            counters[2] != 0
        )?;
        for (name, value) in [
            ("dev_equal", record[3]),
            ("ino_equal", record[4]),
            ("mount_equal", record[5]),
        ] {
            let value = match value {
                0 => "false",
                1 => "true",
                _ => "null",
            };
            write!(output, ",\"{name}\":{value}")?;
        }
        writeln!(output, ",\"returned_category\":\"{category}\"}}")
    }

    fn emit(&self) {
        let Some(failure) = self.native_failure.get() else {
            return;
        };
        if self.observed_context.is_none() {
            return;
        }
        let Some(record) = SLOT.with(Cell::get) else {
            return;
        };
        let returned = if std::thread::panicking() {
            8
        } else {
            self.returned_category.get()
        };
        #[cfg(test)]
        EMITTED.with(|slot| {
            let tag = match failure {
                Failure::BudgetExceeded => 1,
                Failure::Timeout => 2,
                Failure::Cancelled => 3,
                Failure::PermissionDenied => 4,
                Failure::Conflict => 5,
                Failure::Unsupported => 6,
                Failure::Unavailable => 7,
                Failure::InternalError => 8,
            };
            slot.set(Some([
                record[0],
                record[1],
                record[2],
                record[3],
                record[4],
                record[5],
                tag,
                i64::from(returned),
            ]));
        });
        // 栈内闭合 JSON 只有全部编码成功才写；诊断 I/O 失败不得遮盖原返回或 panic payload。
        let mut storage = [0_u8; 768];
        let mut output = Cursor::new(storage.as_mut_slice());
        let encoded = self.encode(failure, record, returned, &mut output);
        let length = output.position() as usize;
        if encoded.is_ok() {
            let _ = std::io::stderr().lock().write_all(&storage[..length]);
        }
    }
}

impl Drop for ScanValidation493Diagnostic {
    /// 参数：无；返回：无，先尽力记录首错，再恢复同步调用栈的上一层诊断槽。
    fn drop(&mut self) {
        self.emit();
        SLOT.with(|slot| slot.set(self.previous_slot));
        if self.execution {
            CONTEXT.with(|slot| slot.set(self.previous_context));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CONTEXT, EMITTED, SLOT, ScanValidation493Diagnostic};
    use crate::EngineError;
    use crate::native_process::{
        linux_open, linux_scan_namespace::LinuxScanNamespace, linux_scan_root::LinuxScanRoot,
    };
    use diskgraph_core::{BusinessError, ProcessEvidenceFailureCode as Failure};
    use std::cell::{Cell, RefCell};
    use std::ffi::CString;
    use std::io::{Cursor, Error, ErrorKind};
    use std::os::unix::ffi::OsStrExt;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::path::Path;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Instant;
    fn rebind(parent: &Path, root: &Path) {
        let old = parent.with_file_name("old_parent");
        std::fs::rename(parent, &old).unwrap();
        std::fs::create_dir(parent).unwrap();
        std::fs::rename(old.join("root"), root).unwrap();
    }
    fn held_root(parent: &Path, root: &Path) -> LinuxScanRoot {
        std::fs::create_dir_all(root).unwrap();
        let held = LinuxScanRoot::open(root, &|| Ok(())).unwrap().unwrap();
        rebind(parent, root);
        held
    }
    #[test]
    fn first_record_and_validation_lifetimes_are_independent() {
        let _execution = ScanValidation493Diagnostic::execution(Instant::now(), true, true);
        {
            let _first = ScanValidation493Diagnostic::begin();
            ScanValidation493Diagnostic::component(3);
            ScanValidation493Diagnostic::native(1, Some(2));
            ScanValidation493Diagnostic::component(4);
            ScanValidation493Diagnostic::native(2, Some(13));
            assert_eq!(SLOT.with(|slot| slot.get().unwrap()), [1, 2, 3, -1, -1, -1]);
        }
        assert!(SLOT.with(|slot| slot.get()).is_none());
        let _second = ScanValidation493Diagnostic::begin();
        assert_eq!(
            SLOT.with(|slot| slot.get().unwrap()),
            [0, i64::MIN, -1, -1, -1, -1]
        );
        assert_eq!(CONTEXT.with(|slot| slot.get().unwrap().1[0]), 2);
    }
    #[test]
    fn nested_unwind_restores_outer_record_and_original_payload() {
        let _execution = ScanValidation493Diagnostic::execution(Instant::now(), true, true);
        let _outer = ScanValidation493Diagnostic::begin();
        ScanValidation493Diagnostic::native(1, Some(2));
        let sentinel = Box::new(91_u64);
        let address = sentinel.as_ref() as *const u64;
        let caught = catch_unwind(AssertUnwindSafe(|| {
            let _inner_execution =
                ScanValidation493Diagnostic::execution(Instant::now(), false, false);
            let _inner = ScanValidation493Diagnostic::begin();
            ScanValidation493Diagnostic::native(2, Some(13));
            std::panic::resume_unwind(sentinel);
        }));
        let returned = caught.unwrap_err().downcast::<u64>().unwrap();
        assert_eq!(returned.as_ref() as *const u64, address);
        assert_eq!(SLOT.with(|slot| slot.get().unwrap())[1], 2);
        assert_eq!(CONTEXT.with(|slot| slot.get().unwrap().1), [1, 1, 1]);
    }
    #[test]
    fn real_missing_open_preserves_original_class_and_immediate_errno() {
        let temporary = tempfile::tempdir().unwrap();
        let missing =
            CString::new(temporary.path().join("missing").as_os_str().as_bytes()).unwrap();
        let _execution = ScanValidation493Diagnostic::execution(Instant::now(), true, true);
        let _validation = ScanValidation493Diagnostic::begin();
        let error =
            linux_open::open_at(libc::AT_FDCWD, &missing, libc::O_PATH | libc::O_CLOEXEC, 0)
                .unwrap_err();
        assert_eq!(error, Failure::Conflict);
        assert_eq!(
            SLOT.with(|slot| slot.get().unwrap())[1],
            i64::from(libc::ENOENT)
        );
    }
    #[test]
    fn actual_rebinding_is_distinct_from_unchanged_namespace() {
        let temporary = tempfile::tempdir().unwrap();
        let parent = temporary.path().join("parent");
        let root = parent.join("root");
        std::fs::create_dir_all(&root).unwrap();
        let held = LinuxScanNamespace::open(&root, &|| Ok(())).unwrap();
        let _execution = ScanValidation493Diagnostic::execution(Instant::now(), true, true);
        {
            let _validation = ScanValidation493Diagnostic::begin();
            held.verify(&|| Ok(())).unwrap();
            assert_eq!(SLOT.with(|slot| slot.get().unwrap())[0], 0);
        }
        rebind(&parent, &root);
        let _validation = ScanValidation493Diagnostic::begin();
        assert_eq!(held.verify(&|| Ok(())), Err(Failure::Conflict));
        let record = SLOT.with(|slot| slot.get().unwrap());
        assert_eq!(record[0], 4);
        assert_eq!(record[1], i64::MIN);
        assert_eq!(record[4], 0);
    }
    #[test]
    fn real_native_conflict_is_emitted_when_original_postcheck_returns_budget() {
        let temporary = tempfile::tempdir().unwrap();
        let parent = temporary.path().join("parent");
        let root = parent.join("root");
        let held = held_root(&parent, &root);
        let _execution = ScanValidation493Diagnostic::execution(Instant::now(), true, true);
        EMITTED.with(|slot| slot.set(None));
        let result = held.validate(&|| {
            if SLOT.with(|slot| slot.get().is_some_and(|value| value[0] == 4)) {
                Err(BusinessError::BudgetExceeded.into())
            } else {
                Ok(())
            }
        });
        assert!(matches!(
            result,
            Err(EngineError::Business(BusinessError::BudgetExceeded))
        ));
        let record = EMITTED.with(|slot| slot.get().unwrap());
        assert_eq!((record[0], record[4], record[6], record[7]), (4, 0, 5, 2));
        assert!(SLOT.with(Cell::get).is_none());
    }
    #[test]
    fn real_native_conflict_keeps_original_io_postcheck_object_and_single_drop() {
        #[derive(Debug)]
        struct Fault(Arc<AtomicUsize>);
        impl std::fmt::Display for Fault {
            fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                out.write_str("private sentinel")
            }
        }
        impl std::error::Error for Fault {}
        impl Drop for Fault {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let temporary = tempfile::tempdir().unwrap();
        let parent = temporary.path().join("parent");
        let root = parent.join("root");
        let held = held_root(&parent, &root);
        let drops = Arc::new(AtomicUsize::new(0));
        let fault = Box::new(Fault(drops.clone()));
        let address = fault.as_ref() as *const Fault;
        let payload: Box<dyn std::error::Error + Send + Sync> = fault;
        let original = RefCell::new(Some(Error::other(payload)));
        let _execution = ScanValidation493Diagnostic::execution(Instant::now(), true, true);
        EMITTED.with(|slot| slot.set(None));
        let error = held
            .validate(&|| {
                if SLOT.with(|slot| slot.get().is_some_and(|value| value[0] == 4)) {
                    Err(original.borrow_mut().take().unwrap().into())
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        let EngineError::Io(io) = &error else {
            panic!("original I/O must remain primary")
        };
        assert_eq!(
            io.get_ref().unwrap().downcast_ref::<Fault>().unwrap() as *const Fault,
            address
        );
        let record = EMITTED.with(|slot| slot.get().unwrap());
        assert_eq!((record[0], record[6], record[7]), (4, 5, 6));
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(error);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn actual_postcheck_unwind_emits_native_first_and_preserves_original_payload() {
        let temporary = tempfile::tempdir().unwrap();
        let parent = temporary.path().join("parent");
        let root = parent.join("root");
        let held = held_root(&parent, &root);
        let original = Box::new(493_u64);
        let address = original.as_ref() as *const u64;
        let payload = RefCell::new(Some(original));
        let _execution = ScanValidation493Diagnostic::execution(Instant::now(), true, true);
        EMITTED.with(|slot| slot.set(None));
        let caught = catch_unwind(AssertUnwindSafe(|| {
            held.validate(&|| {
                if SLOT.with(|slot| slot.get().is_some_and(|value| value[0] == 4)) {
                    std::panic::resume_unwind(payload.borrow_mut().take().unwrap());
                }
                Ok(())
            })
        }));
        let returned = caught.unwrap_err().downcast::<u64>().unwrap();
        assert_eq!(returned.as_ref() as *const u64, address);
        let record = EMITTED.with(|slot| slot.get().unwrap());
        assert_eq!((record[0], record[6], record[7]), (4, 5, 8));
        assert!(SLOT.with(Cell::get).is_none());
    }
    #[test]
    fn drop_without_returned_call_retains_real_native_errno() {
        let temporary = tempfile::tempdir().unwrap();
        let missing =
            CString::new(temporary.path().join("missing").as_os_str().as_bytes()).unwrap();
        let _execution = ScanValidation493Diagnostic::execution(Instant::now(), true, true);
        EMITTED.with(|slot| slot.set(None));
        {
            let validation = ScanValidation493Diagnostic::begin();
            let result =
                linux_open::open_at(libc::AT_FDCWD, &missing, libc::O_PATH | libc::O_CLOEXEC, 0)
                    .map(|_| ());
            validation.observe_native(&result);
            assert_eq!(result, Err(Failure::Conflict));
        }
        let record = EMITTED.with(|slot| slot.get().unwrap());
        assert_eq!(
            (record[0], record[1], record[6], record[7]),
            (1, i64::from(libc::ENOENT), 5, 0)
        );
    }
    #[test]
    fn bounded_encoding_error_does_not_replace_original_io() {
        let _execution = ScanValidation493Diagnostic::execution(Instant::now(), true, true);
        let validation = ScanValidation493Diagnostic::begin();
        let original = Error::from_raw_os_error(libc::EACCES);
        let record = [1, i64::from(libc::ENOENT), 0, -1, -1, -1];
        let mut storage: [u8; 0] = [];
        let mut output = Cursor::new(storage.as_mut_slice());
        let encoding = validation.encode(Failure::Conflict, record, 6, &mut output);
        assert_eq!(encoding.unwrap_err().kind(), ErrorKind::WriteZero);
        assert_eq!(original.raw_os_error(), Some(libc::EACCES));
    }
}
