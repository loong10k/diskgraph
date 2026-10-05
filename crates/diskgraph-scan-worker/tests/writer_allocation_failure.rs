//! PF-06 首个真实 sink 分配失败必须阻止发布；来源：原生 Rust GlobalAlloc/serde。
//! 每案独立子进程；只拒绝序列化线程武装后的一个大分配，其余请求原样交给 System。
use diskgraph_scan_worker::{Frame, FrameWriter, ProtocolBudgetError, ProtocolLimits};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const LARGE: usize = 64 * 1024;
static HITS: AtomicUsize = AtomicUsize::new(0);
static REJECTED_SIZE: AtomicUsize = AtomicUsize::new(0);
static REJECTED_ALIGN: AtomicUsize = AtomicUsize::new(0);
static ELEMENT_ERROR: AtomicBool = AtomicBool::new(false);
thread_local! {
    // 常量、无 Drop 的 TLS 不在分配器内建立堆对象，其他线程永不武装。
    static ARMED: Cell<bool> = const { Cell::new(false) };
}

/// 透明 System 委托及单次真实分配拒绝；来源：std::alloc::GlobalAlloc 契约。
struct AllocationProbe;

fn reject_once(size: usize, align: usize) -> bool {
    if size < LARGE {
        return false;
    }
    if !ARMED
        .try_with(|armed| armed.replace(false))
        .unwrap_or(false)
    {
        return false;
    }
    REJECTED_SIZE.store(size, Ordering::Relaxed);
    REJECTED_ALIGN.store(align, Ordering::Relaxed);
    HITS.fetch_add(1, Ordering::Relaxed);
    true
}

// SAFETY: 仅对一个合格请求返回允许的 null；成功请求、释放及原 Layout 原样委托 System。
unsafe impl GlobalAlloc for AllocationProbe {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if reject_once(layout.size(), layout.align()) {
            return std::ptr::null_mut();
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if reject_once(layout.size(), layout.align()) {
            return std::ptr::null_mut();
        }
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if reject_once(size, layout.align()) {
            // realloc 失败仍由调用者保有原块，不释放或改写它。
            return std::ptr::null_mut();
        }
        unsafe { System.realloc(pointer, layout, size) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: AllocationProbe = AllocationProbe;

fn write_sequence(
    writer: &mut FrameWriter<&mut Vec<u8>>,
    input: &str,
    fail: bool,
) -> io::Result<u64> {
    /// 分配探针内部的借用序列化输入；不在武装窗口中建立大字符串。
    struct Sequence<'a> {
        input: &'a str,
        fail: bool,
    }
    impl serde::Serialize for Sequence<'_> {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeSeq;
            let mut sequence = serializer.serialize_seq(Some(1))?;
            // '[' 已由真实 serializer 写入 sink；仅后续借用大字符串的追加分配可命中。
            ARMED.with(|armed| armed.set(self.fail));
            let element = sequence.serialize_element(self.input);
            ARMED.with(|armed| armed.set(false));
            if self.fail {
                let error = element.expect_err("actual sink try_reserve must fail once");
                assert_eq!(error.to_string(), "frame allocation failed");
                ELEMENT_ERROR.store(true, Ordering::Relaxed);
                // 故意吞掉原真实错误，验证公共 writer 不能因 end 返回 Ok 就发布残片。
            } else {
                element?;
            }
            sequence.end()
        }
    }
    writer.write_payload(&Sequence { input, fail })
}

fn limits() -> ProtocolLimits {
    ProtocolLimits {
        max_frame_bytes: (LARGE * 4) as u64,
        max_stream_bytes: (LARGE * 8) as u64,
        max_nodes: 8,
        max_depth: 4,
    }
}

fn isolated(name: &str, work: impl FnOnce()) {
    const CHILD: &str = "DISKGRAPH_ALLOCATION_FAILURE_CHILD";
    if std::env::var(CHILD).ok().as_deref() == Some(name) {
        work();
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD, name)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    eprintln!("{stdout}");
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("test result: ok. 1 passed"));
}

#[test]
fn swallowed_actual_sink_allocation_failure_never_publishes_or_continues() {
    isolated(
        "swallowed_actual_sink_allocation_failure_never_publishes_or_continues",
        || {
            let input = "x".repeat(LARGE);
            let mut output = Vec::new();
            let (result, written, next);
            {
                let mut writer = FrameWriter::new(&mut output, limits());
                result = write_sequence(&mut writer, &input, true);
                written = writer.bytes_written();
                next = writer.write_frame(&Frame::Cancel);
            }
            let hits = HITS.load(Ordering::Relaxed);
            let size = REJECTED_SIZE.load(Ordering::Relaxed);
            let align = REJECTED_ALIGN.load(Ordering::Relaxed);
            println!(
                "allocation_probe hits={hits} requested={size} align={align} element_error={} result={result:?} bytes={written}",
                ELEMENT_ERROR.load(Ordering::Relaxed)
            );
            assert_eq!(hits, 1);
            assert!((LARGE..=LARGE * 2).contains(&size));
            assert_eq!(align, 1, "actual byte buffer allocation qualification");
            assert!(ELEMENT_ERROR.load(Ordering::Relaxed));
            assert!(!ARMED.with(Cell::get));
            assert!(
                output.is_empty(),
                "first sink failure published bytes: {output:?}"
            );
            assert_eq!(written, 0);
            let error = result.unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::Other);
            assert_eq!(error.to_string(), "frame allocation failed");
            assert!(
                !error
                    .get_ref()
                    .is_some_and(|cause| cause.is::<ProtocolBudgetError>())
            );
            let next = next.unwrap_err();
            assert_eq!(next.kind(), io::ErrorKind::Other);
            assert_eq!(next.to_string(), "frame writer already failed");
        },
    );
}

#[test]
fn unarmed_same_sequence_writes_complete_payload_with_original_budget() {
    isolated(
        "unarmed_same_sequence_writes_complete_payload_with_original_budget",
        || {
            let input = "x".repeat(LARGE);
            let mut output = Vec::new();
            let written;
            {
                let mut writer = FrameWriter::new(&mut output, limits());
                written = write_sequence(&mut writer, &input, false).unwrap();
                assert_eq!(writer.bytes_written(), written);
            }
            assert_eq!(HITS.load(Ordering::Relaxed), 0);
            assert!(!ELEMENT_ERROR.load(Ordering::Relaxed));
            assert!(!ARMED.with(Cell::get));
            assert_eq!(written as usize, output.len());
            let length = u32::from_le_bytes(output[..4].try_into().unwrap()) as usize;
            assert_eq!(length + 4, output.len());
            let decoded: Vec<String> = serde_json::from_slice(&output[4..]).unwrap();
            assert_eq!(decoded, vec![input]);
        },
    );
}
