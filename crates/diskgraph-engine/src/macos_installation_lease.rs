use crate::macos_filesystem_state::MacosFilesystemState;
use crate::macos_install_receipt::MacosInstallReceipt;
use crate::macos_installation_claims::MacosInstallationClaims;
use crate::macos_installation_trust::MacosInstallationTrust;
use crate::macos_open_namespace::MacosOpenNamespace;
use crate::{EngineError, ScanWorkerHostConfig};
use diskgraph_core::BusinessError;
use sha2::{Digest, Sha256};
use std::ffi::{CString, OsStr};
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::FileExt;
use std::time::Instant;

/// 独占原镜像和全祖先目录句柄的安装租约，保留已认证声明与完整当前元数据。
/// 来源：原生 Rust PF-06 macOS 安装原生租约合同；无 Java 对等对象。
/// 目录句柄不会冻结名称；可信发行方仍须保证 active 安装及祖先 namespace 不被替换。
pub(super) struct MacosInstallationLease {
    image: File,
    image_state: MacosFilesystemState,
    ancestors: Vec<(File, MacosFilesystemState)>,
    claims: MacosInstallationClaims,
    expected: ScanWorkerHostConfig,
}

impl MacosInstallationLease {
    /// 先验签，再逐组件原生打开并完整验证原镜像，所有句柄保留在唯一租约中。
    /// 参数：receipt/trust/expected 分别为有界声明、独立信任、独立摘要，检查点借用原期限与权限。
    /// 返回：可进一步绑定 child owner 的安装租约；本方法不启动镜像、不证明历史 writer 已消失。
    pub(super) fn admit(
        receipt: &[u8],
        trust: &MacosInstallationTrust,
        expected: &ScanWorkerHostConfig,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<Self, EngineError> {
        check(deadline, checkpoint)?;
        let claims = MacosInstallReceipt::verify(receipt, trust, expected)?;
        check(deadline, checkpoint)?;
        let (ancestors, image, image_state) =
            open_namespace(&claims.native_path, deadline, checkpoint)?;
        if image_state.mode & 0o111 == 0 {
            return Err(BusinessError::Unsupported.into());
        }
        match_claims(&image_state, &claims)?;
        let lease = Self {
            image,
            image_state,
            ancestors,
            claims,
            expected: *expected,
        };
        lease.revalidate(deadline, checkpoint)?;
        Ok(lease)
    }

    /// 重新验证 held 身份、原 namespace 及完整原 FD 摘要，不能用先前准入延长执行期限。
    /// 参数：deadline/checkpoint 沿用当前 claim；返回：所有绑定仍成立或原错误。
    /// 使用 read_at 不克隆 File、不共享 seek 位置；允许多次独立只读核验同一不可变安装。
    pub(super) fn revalidate(
        &self,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        self.check_held(deadline, checkpoint)?;
        self.check_namespace(deadline, checkpoint)?;
        self.check_digest(deadline, checkpoint)?;
        self.check_held(deadline, checkpoint)?;
        self.check_namespace(deadline, checkpoint)?;
        check(deadline, checkpoint)
    }

    /// 从原安装镜像句柄检查固定加载命令，前后重新核验身份和原namespace。
    /// 参数：deadline/checkpoint沿原请求；返回：策略准入或原失败，不导出File或执行权限。
    pub(super) fn validate_loader(
        &self,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        self.revalidate(deadline, checkpoint)?;
        crate::macos_load_policy::MacosLoadPolicy::validate(
            &self.image,
            self.expected.expected_bytes,
            deadline,
            checkpoint,
        )?;
        self.check_held(deadline, checkpoint)?;
        self.check_namespace(deadline, checkpoint)?;
        check(deadline, checkpoint)
    }

    /// 借用已认证的无损原生镜像路径，仅供宿主后续固定 loader 使用。
    /// 参数：无；返回：声明内原路径，不暴露 File、可克隆句柄或执行权限。
    pub(super) fn native_path(&self) -> &OsStr {
        OsStr::from_bytes(&self.claims.native_path)
    }

    fn check_held(
        &self,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        for (file, original) in &self.ancestors {
            check(deadline, checkpoint)?;
            let current = MacosFilesystemState::capture(file, true)?;
            check(deadline, checkpoint)?;
            if !original.same_directory_binding(&current) {
                return Err(BusinessError::Conflict.into());
            }
        }
        check(deadline, checkpoint)?;
        let current = MacosFilesystemState::capture(&self.image, false)?;
        check(deadline, checkpoint)?;
        if current != self.image_state {
            return Err(BusinessError::Conflict.into());
        }
        match_claims(&current, &self.claims)
    }

    fn check_namespace(
        &self,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        // 额外 FD 仅用于名称当前绑定的身份比较，不代替原 image 消费内容，也不移交给 loader。
        let (current_ancestors, _comparison_image, current_image_state) =
            open_namespace(&self.claims.native_path, deadline, checkpoint)?;
        if current_ancestors.len() != self.ancestors.len()
            || current_image_state != self.image_state
            || current_ancestors
                .iter()
                .zip(&self.ancestors)
                .any(|((_, current), (_, original))| !original.same_directory_binding(current))
        {
            return Err(BusinessError::Conflict.into());
        }
        check(deadline, checkpoint)
    }

    fn check_digest(
        &self,
        deadline: Instant,
        checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        let mut offset = 0_u64;
        while offset < self.expected.expected_bytes {
            check(deadline, checkpoint)?;
            let length = (self.expected.expected_bytes - offset).min(buffer.len() as u64) as usize;
            let read = self.image.read_at(&mut buffer[..length], offset)?;
            check(deadline, checkpoint)?;
            if read == 0 {
                return Err(BusinessError::Conflict.into());
            }
            hasher.update(&buffer[..read]);
            offset += read as u64;
        }
        // 完整预算之外只做一字节 EOF 探针，增长文件不得返回可被当作完整摘要的结果。
        check(deadline, checkpoint)?;
        let mut excess = [0_u8; 1];
        let read = self.image.read_at(&mut excess, offset)?;
        check(deadline, checkpoint)?;
        let digest: [u8; 32] = hasher.finalize().into();
        if read != 0 || digest != self.expected.expected_sha256 {
            return Err(BusinessError::Conflict.into());
        }
        Ok(())
    }
}

fn check(
    deadline: Instant,
    checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    // 保留原检查点错误对象，不格式化成字符串或用新错误覆盖撤权/取消原因。
    checkpoint()?;
    if Instant::now() >= deadline {
        return Err(BusinessError::BudgetExceeded.into());
    }
    Ok(())
}

fn match_claims(
    state: &MacosFilesystemState,
    claims: &MacosInstallationClaims,
) -> Result<(), EngineError> {
    if state.volume_uuid != claims.volume_uuid
        || state.fsid != claims.fsid
        || state.device != claims.device
        || state.inode != claims.inode
        || state.birth_seconds != claims.birth_seconds
        || state.birth_nanoseconds != claims.birth_nanoseconds
        || state.len != claims.image_bytes
    {
        return Err(BusinessError::Conflict.into());
    }
    Ok(())
}
/// 参数：path 为可信绝对原生路径，deadline/checkpoint 为原请求期限；返回：持有路径对象与祖先的命名空间，或路径、预算及原生访问错误。
///
pub(super) fn open_namespace(
    path: &[u8],
    deadline: Instant,
    checkpoint: &mut impl FnMut() -> Result<(), EngineError>,
) -> Result<MacosOpenNamespace, EngineError> {
    if !MacosInstallationTrust::is_valid_path(path) {
        return Err(BusinessError::InvalidArgument.into());
    }
    check(deadline, checkpoint)?;
    let root_name = c"/";
    // 安全性：常量根路径无 NUL 歧义，原生 FD 成功返回后立即由唯一 File 接管。
    let fd = unsafe {
        libc::open(
            root_name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let root = unsafe { File::from_raw_fd(fd) };
    check(deadline, checkpoint)?;
    let root_state = MacosFilesystemState::capture(&root, true)?;
    check(deadline, checkpoint)?;
    let mut ancestors = vec![(root, root_state)];
    let mut components = path[1..].split(|byte| *byte == b'/').peekable();
    while let Some(component) = components.next() {
        check(deadline, checkpoint)?;
        let name = CString::new(component).map_err(|_| BusinessError::InvalidArgument)?;
        let directory = components.peek().is_some();
        let kind = if directory {
            libc::O_DIRECTORY
        } else {
            libc::O_NONBLOCK
        };
        let parent = &ancestors.last().expect("root remains owned").0;
        // 安全性：单一已校验组件不含 '/' 或 NUL，父 FD 保活；逐层 NOFOLLOW 禁止链接逃逸。
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | kind,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        check(deadline, checkpoint)?;
        let state = MacosFilesystemState::capture(&file, directory)?;
        check(deadline, checkpoint)?;
        if directory {
            ancestors.push((file, state));
        } else {
            return Ok((ancestors, file, state));
        }
    }
    Err(BusinessError::InvalidArgument.into())
}
