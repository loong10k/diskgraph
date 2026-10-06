//! 当前Rust原句柄准入与存活可写视图的真实负控；不操作产品或用户镜像。
use crate::windows_scan_image_lease::WindowsScanImageLease;
use crate::{EngineError, ScanWorkerHostConfig};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::time::{Duration, Instant};
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, PAGE_READWRITE,
    UnmapViewOfFile,
};

/// 实际view唯一所有者；来源：Windows CreateFileMapping/MapViewOfFile，关闭mapping不移除view。
struct RetainedImageView {
    view: MEMORY_MAPPED_VIEW_ADDRESS,
}
impl RetainedImageView {
    fn new(writer: &File) -> Self {
        let raw = unsafe {
            CreateFileMappingW(
                writer.as_raw_handle(),
                std::ptr::null(),
                PAGE_READWRITE,
                0,
                0,
                std::ptr::null(),
            )
        };
        assert!(
            !raw.is_null(),
            "actual mapping failed: {}",
            std::io::Error::last_os_error()
        );
        let mapping = unsafe { OwnedHandle::from_raw_handle(raw) };
        let view = unsafe { MapViewOfFile(mapping.as_raw_handle(), FILE_MAP_WRITE, 0, 0, 4096) };
        assert!(
            !view.Value.is_null(),
            "actual writable view failed: {}",
            std::io::Error::last_os_error()
        );
        let retained = Self { view };
        // 返回前关闭mapping句柄，view仍保有真实内核映射；失败Drop不遗留mapping。
        drop(mapping);
        retained
    }
    fn write_same_header_byte(&self) {
        let byte = self.view.Value.cast::<u8>();
        // 此处在准入前且原writer/mapping句柄均关闭；只对本夹具副本执行真实同字节写。
        unsafe {
            assert_eq!(std::ptr::read_volatile(byte), b'M');
            std::ptr::write_volatile(byte, b'M');
            assert_eq!(std::ptr::read_volatile(byte), b'M');
        }
    }
    fn unmap(&mut self) {
        if !self.view.Value.is_null() {
            assert_ne!(
                unsafe { UnmapViewOfFile(self.view) },
                0,
                "actual unmap failed: {}",
                std::io::Error::last_os_error()
            );
            self.view.Value = std::ptr::null_mut();
        }
    }
}
impl Drop for RetainedImageView {
    fn drop(&mut self) {
        // 断言展开期间只做资源兜底；正常验收已通过显式unmap确认原生成功。
        if !self.view.Value.is_null() {
            let _ = unsafe { UnmapViewOfFile(self.view) };
        }
    }
}

#[test]
fn retained_writable_view_after_both_handles_close_blocks_current_rust_lease() {
    let deadline = Instant::now() + Duration::from_secs(20);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("original.exe");
    let bytes = std::fs::read(std::env::current_exe().unwrap()).unwrap();
    assert!(bytes.len() >= 4096 && bytes[..2] == *b"MZ");
    std::fs::write(&path, &bytes).unwrap();
    let expected = ScanWorkerHostConfig::from_expected_image(
        Sha256::digest(&bytes).into(),
        bytes.len() as u64,
    )
    .unwrap();
    let original = File::open(&path).unwrap();
    let writer = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    let mut view = RetainedImageView::new(&writer);
    drop(writer);
    view.write_same_header_byte();
    eprintln!("DG_CURRENT_RUST_WRITABLE_VIEW_HANDLES_CLOSED=1");
    let result = WindowsScanImageLease::prepare(
        original.try_clone().unwrap(),
        &expected,
        deadline,
        &mut || Ok(()),
    );
    let rejected =
        matches!(&result, Err(EngineError::Io(error)) if error.raw_os_error() == Some(32));
    let admission = match &result {
        Ok(_) => "admitted retained writable view".to_owned(),
        Err(error) => format!("{error:?}"),
    };
    // 准入成功不能被解释成安全；不在潜在保护变化之后再次危险写入。
    drop(result);
    view.unmap();
    let positive = WindowsScanImageLease::prepare(original, &expected, deadline, &mut || Ok(()));
    assert!(
        rejected,
        "current Rust writable-view admission was {admission}"
    );
    positive.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    eprintln!("DG_CURRENT_RUST_VIEW_REJECTED_RELEASED_IMAGE_QUALIFIED=1");
}
