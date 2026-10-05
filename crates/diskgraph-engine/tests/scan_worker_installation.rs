//! 受信宿主的已打开 helper 镜像材料验收；来源：OpenSpec physical-scan-process。
//! 夹具仅写入真实普通文件字节，不执行镜像，不证明子进程或发布许可。

use diskgraph_core::BusinessError;
use diskgraph_engine::{EngineError, ScanWorkerHostConfig, ScanWorkerInstallation};
use sha2::{Digest, Sha256};
use std::fs::{File, FileTimes, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime};

const MAX_IMAGE_BYTES: u64 = 128 << 20;
const IMAGE_BYTES: usize = 512 << 10;

fn bytes() -> Vec<u8> {
    (0..IMAGE_BYTES).map(|index| (index % 251) as u8).collect()
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn fixture() -> (tempfile::TempDir, std::path::PathBuf, Vec<u8>) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("held-image");
    let image = bytes();
    std::fs::write(&path, &image).unwrap();
    (directory, path, image)
}

fn config(image: &[u8]) -> ScanWorkerHostConfig {
    ScanWorkerHostConfig::from_expected_image(digest(image), image.len() as u64).unwrap()
}

fn deadline() -> Instant {
    // 足额阶段资格窗，不是镜像验证的响应时间门槛或新的请求时钟。
    Instant::now() + Duration::from_secs(5)
}

fn error<T>(result: Result<T, EngineError>) -> EngineError {
    match result {
        Ok(_) => panic!("unexpected installation material acceptance"),
        Err(error) => error,
    }
}

fn assert_business<T>(result: Result<T, EngineError>, expected: BusinessError) {
    let error = error(result);
    assert!(
        matches!(&error, EngineError::Business(actual) if *actual == expected),
        "expected {expected:?}, actual {error:?}"
    );
}

#[cfg(unix)]
fn held_handle(file: &File) -> usize {
    use std::os::fd::AsRawFd;
    file.as_raw_fd() as usize
}

#[cfg(windows)]
fn held_handle(file: &File) -> usize {
    use std::os::windows::io::AsRawHandle;
    file.as_raw_handle() as usize
}

fn assert_original_file(material: ScanWorkerInstallation, handle: usize, image: &[u8]) {
    assert_eq!(material.image_sha256(), digest(image));
    assert_eq!(material.image_bytes(), image.len() as u64);
    let mut held = material.into_file();
    assert_eq!(
        held_handle(&held),
        handle,
        "must transfer the original held File"
    );
    held.seek(SeekFrom::Start(0)).unwrap();
    let mut actual = Vec::new();
    held.read_to_end(&mut actual).unwrap();
    assert_eq!(actual, image);
}

#[test]
fn complete_real_image_returns_the_same_held_file_and_full_digest() {
    let (_directory, path, image) = fixture();
    let mut file = File::open(path).unwrap();
    // 核验必须定位完整镜像，不能把调用者当前游标后的后缀当完整材料。
    file.seek(SeekFrom::Start(17)).unwrap();
    let handle = held_handle(&file);
    let mut checkpoints = 0;
    let result = ScanWorkerInstallation::verify(file, &config(&image), deadline(), || {
        checkpoints += 1;
        Ok(())
    });
    assert!(checkpoints > 0, "the actual checkpoint must be used");
    assert_original_file(result.unwrap(), handle, &image);
}

#[test]
fn expected_image_configuration_rejects_zero_and_over_hard_limit() {
    assert_business(
        ScanWorkerHostConfig::from_expected_image([0; 32], 0),
        BusinessError::InvalidArgument,
    );
    assert_business(
        ScanWorkerHostConfig::from_expected_image([0; 32], MAX_IMAGE_BYTES + 1),
        BusinessError::BudgetExceeded,
    );
    assert!(ScanWorkerHostConfig::from_expected_image([0; 32], MAX_IMAGE_BYTES).is_ok());
}

#[test]
fn wrong_digest_and_same_size_changed_content_are_conflicts() {
    let (_directory, path, image) = fixture();
    let mut wrong = digest(&image);
    wrong[0] ^= 1;
    let expected = ScanWorkerHostConfig::from_expected_image(wrong, image.len() as u64).unwrap();
    assert_business(
        ScanWorkerInstallation::verify(
            File::open(&path).unwrap(),
            &expected,
            deadline(),
            || Ok(()),
        ),
        BusinessError::Conflict,
    );
    let mut changed = image.clone();
    changed[0] ^= 1;
    std::fs::write(&path, &changed).unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().len(), image.len() as u64);
    assert_business(
        ScanWorkerInstallation::verify(
            File::open(path).unwrap(),
            &config(&image),
            deadline(),
            || Ok(()),
        ),
        BusinessError::Conflict,
    );
}

#[test]
fn shorter_and_longer_real_images_are_conflicts() {
    let (_directory, path, image) = fixture();
    for expected_bytes in [image.len() as u64 - 1, image.len() as u64 + 1] {
        let expected =
            ScanWorkerHostConfig::from_expected_image(digest(&image), expected_bytes).unwrap();
        assert_business(
            ScanWorkerInstallation::verify(
                File::open(&path).unwrap(),
                &expected,
                deadline(),
                || Ok(()),
            ),
            BusinessError::Conflict,
        );
    }
}

#[cfg(unix)]
fn directory_file(path: &Path) -> File {
    File::open(path).unwrap()
}

#[cfg(windows)]
fn directory_file(path: &Path) -> File {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS;
    OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .unwrap()
}

#[test]
fn directory_handle_is_not_an_installable_ordinary_file() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory_file(directory.path());
    assert!(file.metadata().unwrap().is_dir());
    let expected = ScanWorkerHostConfig::from_expected_image([0; 32], 1).unwrap();
    assert_business(
        ScanWorkerInstallation::verify(file, &expected, deadline(), || Ok(())),
        BusinessError::InvalidArgument,
    );
}

#[test]
fn oversized_sparse_write_only_file_is_rejected_before_any_read() {
    let directory = tempfile::tempdir().unwrap();
    // 此真实句柄只有写权限：读取将产生原生 I/O 错误，因此 Budget 证明先做硬准入。
    let file = File::create(directory.path().join("sparse-image")).unwrap();
    file.set_len(MAX_IMAGE_BYTES + 1).unwrap();
    assert_eq!(file.metadata().unwrap().len(), MAX_IMAGE_BYTES + 1);
    let expected = ScanWorkerHostConfig::from_expected_image([0; 32], MAX_IMAGE_BYTES).unwrap();
    assert_business(
        ScanWorkerInstallation::verify(file, &expected, deadline(), || Ok(())),
        BusinessError::BudgetExceeded,
    );
}

#[test]
fn already_spent_original_deadline_is_not_renewed() {
    let (_directory, path, image) = fixture();
    let original = Instant::now()
        .checked_sub(Duration::from_millis(1))
        .unwrap();
    assert_business(
        ScanWorkerInstallation::verify(
            File::open(path).unwrap(),
            &config(&image),
            original,
            || Ok(()),
        ),
        BusinessError::BudgetExceeded,
    );
}

#[test]
fn deadline_spent_inside_actual_checkpoint_still_rejects_material() {
    let (_directory, path, image) = fixture();
    let original = Instant::now() + Duration::from_secs(1);
    let mut entered_live = false;
    let result = ScanWorkerInstallation::verify(
        File::open(path).unwrap(),
        &config(&image),
        original,
        || {
            if !entered_live {
                entered_live = Instant::now() < original;
                let remaining = original.saturating_duration_since(Instant::now());
                std::thread::sleep(remaining + Duration::from_millis(10));
            }
            Ok(())
        },
    );
    assert!(
        entered_live,
        "actual checkpoint qualification must precede original expiry"
    );
    assert_business(result, BusinessError::BudgetExceeded);
}

#[test]
fn rename_before_identity_capture_does_not_reopen_the_replacement_path() {
    let (directory, path, image) = fixture();
    let file = File::open(&path).unwrap();
    let handle = held_handle(&file);
    let mut moved = false;
    let result = ScanWorkerInstallation::verify(file, &config(&image), deadline(), || {
        if !moved {
            // 首检查点在身份采集前；之后严格比较采集前后的 change 信息。
            std::fs::rename(&path, directory.path().join("original-renamed"))?;
            std::fs::write(&path, vec![255; image.len()])?;
            moved = true;
        }
        Ok(())
    });
    assert!(moved, "the real rename and replacement must occur");
    assert_ne!(std::fs::read(path).unwrap(), image);
    assert_original_file(result.unwrap(), handle, &image);
}

#[test]
fn rename_after_actual_hash_read_changes_metadata_and_is_a_conflict() {
    let (directory, path, image) = fixture();
    let file = File::open(&path).unwrap();
    let mut cursor_witness = file.try_clone().unwrap();
    let mut moved = false;
    let result = ScanWorkerInstallation::verify(file, &config(&image), deadline(), || {
        if !moved && cursor_witness.stream_position()? > 0 {
            std::fs::rename(&path, directory.path().join("renamed-during-hash"))?;
            moved = true;
        }
        Ok(())
    });
    assert!(moved, "must reach a checkpoint after an actual image read");
    assert_business(result, BusinessError::Conflict);
}

#[test]
fn actual_truncate_during_hash_cannot_return_a_complete_digest() {
    let (_directory, path, image) = fixture();
    let file = File::open(&path).unwrap();
    let mut cursor_witness = file.try_clone().unwrap();
    let mut truncated = false;
    let result = ScanWorkerInstallation::verify(file, &config(&image), deadline(), || {
        if !truncated && cursor_witness.stream_position()? > 0 {
            OpenOptions::new().write(true).open(&path)?.set_len(1)?;
            truncated = true;
        }
        Ok(())
    });
    assert!(truncated, "must truncate after an actual hash read");
    assert_eq!(std::fs::metadata(path).unwrap().len(), 1);
    assert_business(result, BusinessError::Conflict);
}

#[test]
fn in_place_change_of_already_read_prefix_is_rejected_by_terminal_metadata() {
    let (_directory, path, image) = fixture();
    let mut writer = OpenOptions::new().write(true).open(&path).unwrap();
    let before = SystemTime::UNIX_EPOCH + Duration::from_secs(1_750_000_000);
    writer
        .set_times(FileTimes::new().set_modified(before))
        .unwrap();
    let file = File::open(&path).unwrap();
    let mut cursor_witness = file.try_clone().unwrap();
    let captured_modified = file.metadata().unwrap().modified().unwrap();
    let mut changed = false;
    let result = ScanWorkerInstallation::verify(file, &config(&image), deadline(), || {
        if !changed && cursor_witness.stream_position()? > 0 {
            // 改写已读前缀，长度不变；摘要本身不能代替末段 change/修改信息检查。
            writer.seek(SeekFrom::Start(0))?;
            writer.write_all(&[image[0] ^ 1])?;
            writer.set_times(FileTimes::new().set_modified(before + Duration::from_millis(1)))?;
            writer.sync_all()?;
            changed = true;
        }
        Ok(())
    });
    assert!(
        changed,
        "must change the already read prefix in the real file"
    );
    let metadata = writer.metadata().unwrap();
    assert_eq!(metadata.len(), image.len() as u64);
    let actual_modified = metadata.modified().unwrap();
    assert_ne!(
        actual_modified, captured_modified,
        "high precision timestamp qualification"
    );
    assert_eq!(
        actual_modified
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        captured_modified
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        "only subsecond modification time changed"
    );
    assert_business(result, BusinessError::Conflict);
}

#[test]
fn real_read_failure_preserves_original_native_io_kind_and_errno() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("write-only-image");
    let image = b"real write-only image";
    let mut file = File::create(path).unwrap();
    file.write_all(image).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    let native = file.read(&mut [0; 1]).unwrap_err();
    let result = ScanWorkerInstallation::verify(file, &config(image), deadline(), || Ok(()));
    match error(result) {
        EngineError::Io(actual) => {
            assert_eq!(actual.kind(), native.kind());
            assert_eq!(actual.raw_os_error(), native.raw_os_error());
            assert!(actual.raw_os_error().is_some());
        }
        other => panic!("original native I/O error expected, actual {other:?}"),
    }
}

/// 非 Clone 的实际检查点错误载荷；来源：Rust 原错误所有权契约。
#[derive(Debug)]
struct UniqueCheckpointFault {
    drops: Arc<AtomicUsize>,
}

impl std::fmt::Display for UniqueCheckpointFault {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output.write_str("installation-checkpoint-original-payload")
    }
}

impl std::error::Error for UniqueCheckpointFault {}

impl Drop for UniqueCheckpointFault {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn checkpoint_failure_returns_the_same_nonclone_payload_without_reconstruction() {
    let (_directory, path, image) = fixture();
    let drops = Arc::new(AtomicUsize::new(0));
    let payload = Box::new(UniqueCheckpointFault {
        drops: drops.clone(),
    });
    let original = payload.as_ref() as *const UniqueCheckpointFault as usize;
    // 先擦除为实际错误对象；泛型 Box<UniqueCheckpointFault> 否则会再被装箱为载荷。
    let payload: Box<dyn std::error::Error + Send + Sync> = payload;
    let supplied_io = io::Error::other(payload);
    let admitted = supplied_io
        .get_ref()
        .unwrap()
        .downcast_ref::<UniqueCheckpointFault>()
        .expect("the original supplied payload must qualify before verification");
    assert_eq!(admitted as *const UniqueCheckpointFault as usize, original);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    let mut supplied = Some(EngineError::Io(supplied_io));
    let result = ScanWorkerInstallation::verify(
        File::open(path).unwrap(),
        &config(&image),
        deadline(),
        || {
            Err(supplied
                .take()
                .expect("checkpoint failure must stop further callbacks"))
        },
    );
    assert!(
        supplied.is_none(),
        "actual checkpoint must consume its original error"
    );
    let returned = error(result);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    match &returned {
        EngineError::Io(actual) => {
            let payload = actual
                .get_ref()
                .unwrap()
                .downcast_ref::<UniqueCheckpointFault>()
                .unwrap();
            assert_eq!(payload as *const UniqueCheckpointFault as usize, original);
            assert_eq!(actual.kind(), io::ErrorKind::Other);
        }
        other => panic!("original checkpoint I/O payload expected, actual {other:?}"),
    }
    drop(returned);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
