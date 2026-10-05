//! 仅固定 ae 隔离构建的验证首错观察；不挂入主生产树、不改变原 syscall 或错误分类。
use diskgraph_core::ProcessEvidenceFailureCode as Failure;
use std::cell::Cell;
use std::io::{Cursor, Write};
use std::time::Instant;

thread_local! {
    // 原执行时钟、调用序号、原 owner/principal 比较结果；不保存或输出身份字符串。
    static CONTEXT: Cell<Option<(Instant, [u64; 3])>> = const { Cell::new(None) };
    // operation、errno、组件序号、dev/ino/mount 比较结果；固定内存，无历史列表。
    static SLOT: Cell<Option<[i64; 6]>> = const { Cell::new(None) };
}

/// 当前执行或当前一次 validate 的有界诊断租约；来源：DiskGraph 原生 Rust Linux 诊断实验。
/// 仅保存失败标量；TLS 只定位同步调用栈，不共享 Engine、数据库、授权或允许缓存。
pub(crate) struct ScanValidationDiagnostic {
    execution: bool,
    previous_context: Option<(Instant, [u64; 3])>,
    previous_slot: Option<[i64; 6]>,
    observed_context: Option<(Instant, [u64; 3])>,
}

impl ScanValidationDiagnostic {
    /// 参数：started 是原 scan_started；两个布尔值来自本次真实 job 与 benchmark 请求比较。
    /// 返回：执行局部租约，退出或 unwind 后恢复上一层，绝不输出 job/scope/principal 值。
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
        }
    }

    /// 参数：无；返回：当前一次原生验证的空首错槽；上一验证的 errno 不得流入本次。
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
        }
    }

    /// 参数：index 是当前锚零或祖先组件序号；返回：无，首错记录后不修改其位置。
    pub(super) fn component(index: usize) {
        SLOT.with(|slot| {
            if let Some(mut record) = slot.get().filter(|value| value[0] == 0) {
                record[2] = i64::try_from(index).unwrap_or(i64::MAX);
                slot.set(Some(record));
            }
        });
    }

    /// 参数：operation 为固定 1=openat2、2=statx、3=fstat；errno 是失败发生处立即捕获值。
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

    /// 参数：三个布尔值为原持有对象与当前真实对象的比较；返回：无，不保存具体 ID。
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

    /// 参数：result 为原 checked_native 成功返回且原末次 check 已通过后的真实原生结果。
    /// 返回：无；成功不打印，失败只写固定有界标量，诊断写入失败不替换原结果。
    pub(super) fn finish(&self, result: &Result<(), Failure>) {
        let Err(failure) = result else {
            return;
        };
        let Some((started, counters)) = self.observed_context else {
            return;
        };
        let Some(record) = SLOT.with(Cell::get) else {
            return;
        };
        let operation = match record[0] {
            1 => "openat2",
            2 => "statx",
            3 => "fstat",
            4 => "identity",
            _ => "unknown",
        };
        // 一个栈缓冲内完成闭合 JSON；只有完整编码成功才尝试一次 stderr 写入。
        let mut storage = [0_u8; 768];
        let mut output = Cursor::new(storage.as_mut_slice());
        let encoded = (|| -> std::io::Result<()> {
            write!(
                output,
                "DG_SCAN_VALIDATION_FIRST_FAILURE {{\"phase\":\"namespace_validate\",\"operation\":\"{operation}\",\"native_failure\":\"{}\",\"errno\":",
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
            writeln!(output, "}}")
        })();
        let length = output.position() as usize;
        if encoded.is_ok() {
            let _ = std::io::stderr().lock().write_all(&storage[..length]);
        }
    }
}

impl Drop for ScanValidationDiagnostic {
    fn drop(&mut self) {
        SLOT.with(|slot| slot.set(self.previous_slot));
        if self.execution {
            CONTEXT.with(|slot| slot.set(self.previous_context));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CONTEXT, SLOT, ScanValidationDiagnostic};
    use crate::native_process::{linux_open, linux_scan_namespace::LinuxScanNamespace};
    use diskgraph_core::ProcessEvidenceFailureCode as Failure;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::time::Instant;

    #[test]
    fn first_record_and_validation_lifetimes_are_independent() {
        let _execution = ScanValidationDiagnostic::execution(Instant::now(), true, true);
        {
            let _first = ScanValidationDiagnostic::begin();
            ScanValidationDiagnostic::component(3);
            ScanValidationDiagnostic::native(1, Some(2));
            ScanValidationDiagnostic::component(4);
            ScanValidationDiagnostic::native(2, Some(13));
            assert_eq!(SLOT.with(|slot| slot.get().unwrap()), [1, 2, 3, -1, -1, -1]);
        }
        assert!(SLOT.with(|slot| slot.get()).is_none());
        let _second = ScanValidationDiagnostic::begin();
        assert_eq!(
            SLOT.with(|slot| slot.get().unwrap()),
            [0, i64::MIN, -1, -1, -1, -1]
        );
        assert_eq!(CONTEXT.with(|slot| slot.get().unwrap().1[0]), 2);
    }

    #[test]
    fn nested_unwind_restores_outer_record_and_original_payload() {
        let _execution = ScanValidationDiagnostic::execution(Instant::now(), true, true);
        let _outer = ScanValidationDiagnostic::begin();
        ScanValidationDiagnostic::native(1, Some(2));
        let sentinel = Box::new(91_u64);
        let address = sentinel.as_ref() as *const u64;
        let caught = catch_unwind(AssertUnwindSafe(|| {
            let _inner_execution =
                ScanValidationDiagnostic::execution(Instant::now(), false, false);
            let _inner = ScanValidationDiagnostic::begin();
            ScanValidationDiagnostic::native(2, Some(13));
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
        let _execution = ScanValidationDiagnostic::execution(Instant::now(), true, true);
        let _validation = ScanValidationDiagnostic::begin();
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
        let _execution = ScanValidationDiagnostic::execution(Instant::now(), true, true);
        {
            let _validation = ScanValidationDiagnostic::begin();
            held.verify(&|| Ok(())).unwrap();
            assert_eq!(SLOT.with(|slot| slot.get().unwrap())[0], 0);
        }
        let old_parent = temporary.path().join("old_parent");
        std::fs::rename(&parent, &old_parent).unwrap();
        std::fs::create_dir(&parent).unwrap();
        std::fs::rename(old_parent.join("root"), &root).unwrap();
        let _validation = ScanValidationDiagnostic::begin();
        assert_eq!(held.verify(&|| Ok(())), Err(Failure::Conflict));
        let record = SLOT.with(|slot| slot.get().unwrap());
        assert_eq!(record[0], 4);
        assert_eq!(record[1], i64::MIN);
        assert_eq!(record[4], 0);
    }
}
