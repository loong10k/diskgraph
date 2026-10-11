use crate::{EngineError, ScanWorkerHostConfig};
use diskgraph_core::BusinessError;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::time::Instant;

/// 在监督数据库出生前核验原镜像的全部有界字节；原生 PF-06，无 Java 对等对象。
/// 参数：image 为原 held 普通文件，expected 为独立 root 签发材料，deadline 为原启动期限。
/// 返回：完整摘要及高精度原对象观察一致才成功；不刷新期限、不修改文件、不启动工作。
pub(crate) fn verify_supervisor_birth_image(
    image: &mut File,
    expected: &ScanWorkerHostConfig,
    deadline: Instant,
) -> Result<(), EngineError> {
    let check = || {
        if Instant::now() >= deadline {
            Err(EngineError::from(BusinessError::BudgetExceeded))
        } else {
            Ok(())
        }
    };
    check()?;
    let before = image.metadata()?;
    check()?;
    if !before.is_file() {
        return Err(BusinessError::Unsupported.into());
    }
    if before.len() != expected.expected_bytes {
        return Err(BusinessError::Conflict.into());
    }
    // 不以预期长度无限分配；每次只读剩余额度内的数据，EOF 提前到达不确认完整摘要。
    let mut remaining = expected.expected_bytes;
    let mut buffer = [0_u8; 64 << 10];
    let mut digest = Sha256::new();
    image.seek(SeekFrom::Start(0))?;
    while remaining != 0 {
        check()?;
        let length = remaining.min(buffer.len() as u64) as usize;
        let read = match image.read(&mut buffer[..length]) {
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        check()?;
        if read == 0 {
            return Err(BusinessError::Conflict.into());
        }
        remaining -= read as u64;
        digest.update(&buffer[..read]);
    }
    let after = image.metadata()?;
    check()?;
    let identity = |m: &std::fs::Metadata| {
        (
            m.dev(),
            m.ino(),
            m.len(),
            m.mtime(),
            m.mtime_nsec(),
            m.ctime(),
            m.ctime_nsec(),
        )
    };
    if identity(&before) != identity(&after)
        || <[u8; 32]>::from(digest.finalize()) != expected.expected_sha256
    {
        return Err(BusinessError::Conflict.into());
    }
    image.seek(SeekFrom::Start(0))?;
    check()
}

#[cfg(test)]
mod tests {
    use super::verify_supervisor_birth_image;
    use crate::{EngineError, ScanWorkerHostConfig};
    use diskgraph_core::BusinessError;
    use sha2::{Digest, Sha256};
    use std::io::{Seek, SeekFrom};
    use std::time::{Duration, Instant};

    #[test]
    fn full_original_image_is_verified_and_rewound() {
        let mut image = tempfile::tempfile().unwrap();
        std::io::Write::write_all(&mut image, &vec![7; 130_000]).unwrap();
        let expected = ScanWorkerHostConfig::from_expected_image(
            Sha256::digest(vec![7; 130_000]).into(),
            130_000,
        )
        .unwrap();
        verify_supervisor_birth_image(
            &mut image,
            &expected,
            Instant::now() + Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(image.stream_position().unwrap(), 0);
    }

    #[test]
    fn wrong_same_length_image_refuses_even_without_spawning_a_worker() {
        let mut image = tempfile::tempfile().unwrap();
        std::io::Write::write_all(&mut image, b"abcd").unwrap();
        let expected =
            ScanWorkerHostConfig::from_expected_image(Sha256::digest(b"abce").into(), 4).unwrap();
        assert!(matches!(
            verify_supervisor_birth_image(
                &mut image,
                &expected,
                Instant::now() + Duration::from_secs(2)
            ),
            Err(EngineError::Business(BusinessError::Conflict))
        ));
    }

    #[test]
    fn expired_original_deadline_does_not_read_or_seek_the_image() {
        let mut image = tempfile::tempfile().unwrap();
        std::io::Write::write_all(&mut image, b"abcd").unwrap();
        image.seek(SeekFrom::Start(2)).unwrap();
        let expected =
            ScanWorkerHostConfig::from_expected_image(Sha256::digest(b"abcd").into(), 4).unwrap();
        assert!(matches!(
            verify_supervisor_birth_image(&mut image, &expected, Instant::now()),
            Err(EngineError::Business(BusinessError::BudgetExceeded))
        ));
        assert_eq!(image.stream_position().unwrap(), 2);
    }
}
