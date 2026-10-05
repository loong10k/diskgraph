//! Linux真实sealed-copy边界；来源：PF-06。缺memfd能力必须失败，不跳过。
//! 此组不执行ELF、不认证动态装载环境或子进程退出。
use super::linux_scan_image::LinuxScanImage;
use super::linux_scan_image_error::LinuxScanImageError;
use super::linux_scan_image_fixture::{
    LinuxScanImageFixture, assert_business, assert_sealed_bytes, deadline, failure,
};
use crate::{EngineError, ScanWorkerHostConfig};
use diskgraph_core::BusinessError;
use std::fs::{File, FileTimes, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileExt, MetadataExt};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

#[test]
fn complete_copy_has_real_four_seals_and_rejects_write_grow_shrink() {
    let f = LinuxScanImageFixture::new();
    let mut source = f.open();
    source.seek(SeekFrom::Start(17)).unwrap();
    let identity = source.metadata().unwrap();
    let mut checks = 0;
    let material = LinuxScanImage::prepare(source, &f.config(), deadline(), || {
        checks += 1;
        Ok(())
    })
    .unwrap();
    assert!(
        checks >= f.bytes.len().div_ceil(64 * 1024),
        "must check each bounded block"
    );
    let mut sealed = material.into_file();
    let copied = sealed.metadata().unwrap();
    assert_ne!(
        (identity.dev(), identity.ino()),
        (copied.dev(), copied.ino())
    );
    assert_sealed_bytes(&mut sealed, &f.bytes);
    // 返回的是可读写memfd，但内核seals阻止写入；不能以只读打开的EBADF冒充seal。
    let flags = unsafe { libc::fcntl(sealed.as_raw_fd(), libc::F_GETFL) };
    assert_eq!(flags & libc::O_ACCMODE, libc::O_RDWR);
    assert_eq!(
        sealed.write_at(b"X", 0).unwrap_err().raw_os_error(),
        Some(libc::EPERM)
    );
    assert_eq!(
        sealed
            .set_len(f.bytes.len() as u64 - 1)
            .unwrap_err()
            .raw_os_error(),
        Some(libc::EPERM)
    );
    assert_eq!(
        sealed
            .set_len(f.bytes.len() as u64 + 1)
            .unwrap_err()
            .raw_os_error(),
        Some(libc::EPERM)
    );
    assert_sealed_bytes(&mut sealed, &f.bytes);
}

#[test]
fn prepared_copy_survives_original_in_place_rewrite_and_path_replacement() {
    let f = LinuxScanImageFixture::new();
    let source = f.open();
    let writer = OpenOptions::new().write(true).open(&f.path).unwrap();
    let mut sealed = LinuxScanImage::prepare(source, &f.config(), deadline(), || Ok(()))
        .unwrap()
        .into_file();
    writer.write_at(b"changed-source", 0).unwrap();
    writer.sync_all().unwrap();
    std::fs::rename(&f.path, f.directory.path().join("moved-source")).unwrap();
    std::fs::write(&f.path, b"replacement-content").unwrap();
    assert_ne!(std::fs::read(&f.path).unwrap(), f.bytes);
    assert_sealed_bytes(&mut sealed, &f.bytes);
}

#[test]
fn replacement_before_initial_capture_uses_original_open_file() {
    let f = LinuxScanImageFixture::new();
    let source = f.open();
    let old = source.metadata().unwrap();
    let mut replaced = false;
    let prepared = LinuxScanImage::prepare(source, &f.config(), deadline(), || {
        if !replaced {
            std::fs::rename(&f.path, f.directory.path().join("held-source"))?;
            std::fs::write(&f.path, vec![255; f.bytes.len()])?;
            replaced = true;
        }
        Ok(())
    });
    assert!(replaced);
    assert_ne!(std::fs::metadata(&f.path).unwrap().ino(), old.ino());
    assert_sealed_bytes(&mut prepared.unwrap().into_file(), &f.bytes);
}

#[test]
fn already_copied_source_prefix_changed_in_checkpoint_is_conflict() {
    let f = LinuxScanImageFixture::new();
    let source = f.open();
    let mut cursor = source.try_clone().unwrap();
    let writer = OpenOptions::new().write(true).open(&f.path).unwrap();
    let modified = writer.metadata().unwrap().modified().unwrap();
    let mut changed_at = None;
    let actual = LinuxScanImage::prepare(source, &f.config(), deadline(), || {
        let position = cursor.stream_position()?;
        if changed_at.is_none() && position > 0 {
            writer.write_at(&[f.bytes[0] ^ 1], 0)?;
            writer.set_times(FileTimes::new().set_modified(modified + Duration::from_secs(1)))?;
            writer.sync_all()?;
            changed_at = Some(position);
        }
        Ok(())
    });
    assert!(
        changed_at.is_some(),
        "actual source read must precede mutation"
    );
    assert_eq!(writer.metadata().unwrap().len(), f.bytes.len() as u64);
    assert_ne!(writer.metadata().unwrap().modified().unwrap(), modified);
    assert_business(actual, BusinessError::Conflict);
}

#[test]
fn path_rebound_during_copy_cannot_change_the_source_silently() {
    let f = LinuxScanImageFixture::new();
    let source = f.open();
    let mut cursor = source.try_clone().unwrap();
    let mut replaced = false;
    let actual = LinuxScanImage::prepare(source, &f.config(), deadline(), || {
        if !replaced && cursor.stream_position()? > 0 {
            std::fs::rename(&f.path, f.directory.path().join("during-copy"))?;
            std::fs::write(&f.path, vec![255; f.bytes.len()])?;
            // 确定记录原句柄版本变化，避免以文件系统ctime时钟粒度偶合充资格。
            cursor.set_times(
                FileTimes::new()
                    .set_modified(std::time::SystemTime::now() + Duration::from_secs(2)),
            )?;
            replaced = true;
        }
        Ok(())
    });
    assert!(replaced);
    assert_business(actual, BusinessError::Conflict);
}

#[test]
fn wrong_digest_or_length_never_produces_sealed_material() {
    let f = LinuxScanImageFixture::new();
    let wrong_hash =
        ScanWorkerHostConfig::from_expected_image([0; 32], f.bytes.len() as u64).unwrap();
    assert_business(
        LinuxScanImage::prepare(f.open(), &wrong_hash, deadline(), || Ok(())),
        BusinessError::Conflict,
    );
    for size in [f.bytes.len() as u64 - 1, f.bytes.len() as u64 + 1] {
        let wrong_len = ScanWorkerHostConfig::from_expected_image([0; 32], size).unwrap();
        assert_business(
            LinuxScanImage::prepare(f.open(), &wrong_len, deadline(), || Ok(())),
            BusinessError::Conflict,
        );
    }
}

#[test]
fn oversized_write_only_sparse_source_refuses_before_read() {
    let f = LinuxScanImageFixture::new();
    let source = File::create(f.directory.path().join("oversized")).unwrap();
    source.set_len((128 << 20) + 1).unwrap();
    let config = ScanWorkerHostConfig::from_expected_image([0; 32], 128 << 20).unwrap();
    assert_business(
        LinuxScanImage::prepare(source, &config, deadline(), || Ok(())),
        BusinessError::BudgetExceeded,
    );
}

#[test]
fn actual_write_only_read_error_keeps_native_errno() {
    let f = LinuxScanImageFixture::new();
    let mut source = OpenOptions::new().write(true).open(&f.path).unwrap();
    let original = source.read(&mut [0]).unwrap_err();
    let error = failure(LinuxScanImage::prepare(
        source,
        &f.config(),
        deadline(),
        || Ok(()),
    ));
    match error {
        EngineError::Io(actual) => {
            assert_eq!(actual.kind(), original.kind());
            assert_eq!(actual.raw_os_error(), Some(libc::EBADF));
            assert_eq!(actual.raw_os_error(), original.raw_os_error());
        }
        other => panic!("original native read error required: {other:?}"),
    }
}

#[test]
fn original_deadline_expired_before_or_inside_checkpoint_never_refreshes() {
    let f = LinuxScanImageFixture::new();
    let spent = Instant::now() - Duration::from_millis(1);
    assert_business(
        LinuxScanImage::prepare(f.open(), &f.config(), spent, || Ok(())),
        BusinessError::BudgetExceeded,
    );
    let original = Instant::now() + Duration::from_secs(1);
    let mut entered = false;
    let actual = LinuxScanImage::prepare(f.open(), &f.config(), original, || {
        assert!(!entered, "expired checkpoint must not be re-entered");
        entered = Instant::now() < original;
        std::thread::sleep(
            original.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
        );
        Ok(())
    });
    assert!(
        entered,
        "checkpoint must actually begin inside original window"
    );
    assert_business(actual, BusinessError::BudgetExceeded);
}

#[test]
fn real_copy_checkpoint_cancellation_preserves_exact_primary() {
    let f = LinuxScanImageFixture::new();
    let source = f.open();
    let mut cursor = source.try_clone().unwrap();
    let mut cancelled_after_read = false;
    let actual = LinuxScanImage::prepare(source, &f.config(), deadline(), || {
        if cursor.stream_position()? > 0 {
            cancelled_after_read = true;
            return Err(EngineError::Business(BusinessError::Conflict));
        }
        Ok(())
    });
    assert!(cancelled_after_read);
    assert_business(actual, BusinessError::Conflict);
}

#[test]
fn expired_clock_does_not_replace_the_original_nonclone_checkpoint_payload() {
    let f = LinuxScanImageFixture::new();
    let drops = Arc::new(AtomicUsize::new(0));
    let payload = Box::new(LinuxScanImageError {
        drops: drops.clone(),
    });
    let address = payload.as_ref() as *const LinuxScanImageError;
    let boxed: Box<dyn std::error::Error + Send + Sync> = payload;
    let io = io::Error::other(boxed);
    assert!(std::ptr::eq(
        io.get_ref()
            .unwrap()
            .downcast_ref::<LinuxScanImageError>()
            .unwrap(),
        address
    ));
    let mut original = Some(EngineError::Io(io));
    let actual = failure(LinuxScanImage::prepare(
        f.open(),
        &f.config(),
        Instant::now() - Duration::from_millis(1),
        || Err(original.take().unwrap()),
    ));
    match &actual {
        EngineError::Io(error) => assert!(std::ptr::eq(
            error
                .get_ref()
                .unwrap()
                .downcast_ref::<LinuxScanImageError>()
                .unwrap(),
            address
        )),
        other => panic!("original checkpoint payload required: {other:?}"),
    }
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(actual);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
