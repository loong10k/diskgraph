use super::root_installation_path::{check, denied, identity};
use sha2::{Digest, Sha256};
use std::fs::{File, Metadata};
use std::io::{self, Read};
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::Instant;

/// 保留root安装目录链与完整摘要核验后的原镜像。来源：PF-06固定安装准入；无Java对等对象。
/// 原File用于后续原子launcher，不通过路径再次打开镜像；本类型不授予服务出生或签发发行签名。
pub struct HeldInstallationImage {
    image: File,
    directories: Vec<File>,
}

impl HeldInstallationImage {
    /// 参数：path/sha256为受信安装配置，deadline为原期限；返回：原目录链及原镜像或拒绝。
    /// 仅root可使用；全部祖先root所有且不可由组/其他用户写入，镜像单链接且不可被其他身份写入。
    pub fn open(path: &Path, sha256: &str, deadline: Instant) -> io::Result<Self> {
        let (mut image, directories) = super::root_installation_path::open(path, deadline)?;
        let before = image.metadata()?;
        verify_image(&before)?;
        verify_digest(&mut image, sha256, deadline)?;
        let after = image.metadata()?;
        check(deadline)?;
        verify_image(&after)?;
        if identity(&before) != identity(&after) {
            return Err(denied("installation image changed during verification"));
        }
        check(deadline)?;
        Ok(Self { image, directories })
    }

    /// 参数：无；返回：已核验原File，消费原安装对象供固定原子launcher执行。
    /// root目录链已完成逐组件核验；调用方不得把它转换成新路径打开或客户可选择镜像。
    pub fn into_file(self) -> File {
        let Self { image, directories } = self;
        drop(directories);
        image
    }
}

fn verify_image(metadata: &Metadata) -> io::Result<()> {
    if !metadata.is_file()
        || metadata.uid() != 0
        || metadata.nlink() != 1
        || metadata.mode() & 0o022 != 0
        || metadata.mode() & 0o111 == 0
        || metadata.len() == 0
        || metadata.len() > 256 << 20
    {
        return Err(denied("untrusted installation image"));
    }
    Ok(())
}

/// 参数：file为已打开原镜像，expected为完整摘要，deadline为原期限；返回：完整匹配或拒绝。
/// 固定64KiB读取并限制256MiB总量；不签发owner、目录或平台执行资格。
pub(super) fn verify_digest(file: &mut File, expected: &str, deadline: Instant) -> io::Result<()> {
    if expected.len() != 64
        || !expected
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(denied("invalid complete SHA256"));
    }
    let mut digest = Sha256::new();
    let mut buffer = [0; 65536];
    let mut total = 0u64;
    loop {
        check(deadline)?;
        let limit = buffer.len().min(((256u64 << 20) + 1 - total) as usize);
        let count = file.read(&mut buffer[..limit])?;
        check(deadline)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > 256 << 20 {
            return Err(denied("oversized installation image"));
        }
        digest.update(&buffer[..count]);
    }
    if hex::encode(digest.finalize()) != expected {
        return Err(denied("installation SHA256 mismatch"));
    }
    check(deadline)?;
    Ok(())
}
