use super::linux_atomic_child::LinuxAtomicChild;
use super::linux_atomic_exit::LinuxAtomicExit;
use super::linux_atomic_handshake::LinuxAtomicHandshake;
use super::linux_atomic_launcher::validate_material;
use super::linux_atomic_pipes::LinuxAtomicPipes;
use super::linux_atomic_signal_mask::LinuxAtomicSignalMask;
use super::linux_supervisor_abi::{LinuxSupervisorAbi, dg_linux_supervisor_spawn};
use super::linux_supervisor_launch_error::LinuxSupervisorLaunchError;
use super::linux_supervisor_process::LinuxSupervisorProcess;
use super::unix_child_setup::UnixChildSetup;
use super::{ChildError, ChildSpawnError};
use crate::recovery_control::{LinuxControlReader, LinuxExecutorMaterials, LinuxPeerUnixIdentity};
use crate::recovery_slot::{LinuxSupervisorMaterials, LinuxSupervisorPeer, SlotReservation};
use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::{UnixDatagram, UnixStream};
use std::time::Instant;

/// 受信 root 父侧的有界监督出生准备；来源：PF-06 CLONE_PIDFD 与固定 held ELF 合同。
/// 宿主须已验证安装镜像摘要及配置；本接口不从客户路径、命令或消息签发信任。
pub struct LinuxSupervisorLauncher {
    image: File,
    args: Vec<CString>,
    environment: Vec<CString>,
    identity: (u32, u32),
    groups: Vec<u32>,
    executor_protection: bool,
    pipes: LinuxAtomicPipes,
    extra: [OwnedFd; 2],
    worker: Option<OwnedFd>,
    materials: UnixDatagram,
    controls: UnixStream,
}

impl LinuxSupervisorLauncher {
    /// 参数：worker为root原镜像，expected为独立安装预期，deadline为原期限；返回：固定FD7交付准备。
    /// 完整摘要与只读模式先于槽认领；不接收客户路径，不提供业务文件权限绕过能力。
    pub fn with_worker_image(
        mut self,
        worker: File,
        expected: &crate::ScanWorkerHostConfig,
        deadline: Instant,
    ) -> io::Result<Self> {
        if self.worker.is_some() {
            return Err(io::Error::other("worker image already bound"));
        }
        let image = super::linux_held_worker_image::LinuxHeldWorkerImage::verify(
            worker, expected, deadline,
        )
        .map_err(io::Error::other)?
        .into_file();
        self.worker = Some(duplicate(image.as_raw_fd())?);
        Ok(self)
    }
    /// 参数：原核验镜像、固定参数/环境及原连接内核Unix身份；返回：保留辅助组的出生材料。
    /// Ready前按实际客户凭据拒绝可读镜像；仍须验证exec后隐私、LSM上下文和原guardian责任。
    pub fn prepare_user_executor(
        image: File,
        args: Vec<CString>,
        environment: Vec<CString>,
        identity: &LinuxPeerUnixIdentity,
    ) -> io::Result<Self> {
        let policy = unsafe { super::linux_supervisor_abi::dg_linux_executor_kernel_policy() };
        if policy < 0 {
            return Err(io::Error::from_raw_os_error((-policy) as i32));
        }
        let mut prepared =
            Self::prepare(image, args, environment, (identity.uid(), identity.gid()))?;
        prepared.groups = identity.supplementary_groups().to_vec();
        prepared.executor_protection = true;
        Ok(prepared)
    }

    /// 参数：原验证 ELF、固定 args/environment 与专用 UID/GID；返回：全部出生前准备或原错误。
    /// 不建立发行摘要信任、不继承装载环境；准备失败不创建 child 或认领全局槽。
    pub fn prepare(
        image: File,
        args: Vec<CString>,
        environment: Vec<CString>,
        identity: (u32, u32),
    ) -> io::Result<Self> {
        validate_material(&image, &args, &environment).map_err(io::Error::other)?;
        if identity.0 == 0 || identity.1 == 0 || identity.0 == u32::MAX || identity.1 == u32::MAX {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "dedicated non-root UID/GID required",
            ));
        }
        UnixChildSetup::reject_auto_reap().map_err(io::Error::other)?;
        let pipes = LinuxAtomicPipes::new(&image).map_err(io::Error::other)?;
        let (materials, child_materials) = UnixDatagram::pair()?;
        let child_materials = LinuxSupervisorMaterials::prepare_receiver(child_materials)
            .map_err(io::Error::other)?;
        let (controls, child_controls) = UnixStream::pair()?;
        let controls = LinuxControlReader::prepare_receiver(controls).map_err(io::Error::other)?;
        let extra = [
            duplicate(child_materials.as_raw_fd())?,
            duplicate(child_controls.as_raw_fd())?,
        ];
        Ok(Self {
            image,
            args,
            environment,
            identity,
            groups: Vec::new(),
            executor_protection: false,
            pipes,
            extra,
            worker: None,
            materials,
            controls,
        })
    }

    /// 参数：reservation 为出生前全局原锁、session 为原认证会话、deadline 为原绝对期限。
    /// 返回：唯一原 owner；出生后错误保留同一进程和 ACTIVE，调用方必须接管失败中的 owner。
    /// 本接口只提供有界调用组件，不启用远程入口、长运行 MCP 或桌面服务。
    pub fn spawn(
        self,
        reservation: SlotReservation,
        session: [u8; 32],
        deadline: Instant,
    ) -> Result<Box<LinuxSupervisorProcess>, LinuxSupervisorLaunchError> {
        self.spawn_inner(reservation, session, deadline, false, false)
    }

    /// 参数：原全局reservation、session和启动deadline；返回：保留原owner的版本2生命周期监督进程。
    /// deadline仍约束出生/握手/原材料交付；仅后续私有通知跟随原owner寿命，单次poll不等待。
    /// 宿主必须显式选择同版本的受信角色；不得在旧spawn超时后降级或接受客户命令。
    pub fn spawn_lifecycle(
        self,
        reservation: SlotReservation,
        session: [u8; 32],
        deadline: Instant,
    ) -> Result<Box<LinuxSupervisorProcess>, LinuxSupervisorLaunchError> {
        self.spawn_inner(reservation, session, deadline, true, false)
    }

    /// 参数：reservation为guardian原槽，session为原executor请求摘要，deadline为原启动期限。
    /// 返回：原pidfd及容量责任仍在父侧的executor进程；失败返回同一出生后owner。
    /// 必须已绑定完整worker镜像及真实客户凭据；只准备无句柄认证包，禁止旧容量材料交付。
    pub fn spawn_executor_lifecycle(
        self,
        reservation: SlotReservation,
        session: [u8; 32],
        deadline: Instant,
    ) -> Result<Box<LinuxSupervisorProcess>, LinuxSupervisorLaunchError> {
        self.spawn_inner(reservation, session, deadline, true, true)
    }

    fn spawn_inner(
        self,
        reservation: SlotReservation,
        session: [u8; 32],
        deadline: Instant,
        lifecycle: bool,
        executor: bool,
    ) -> Result<Box<LinuxSupervisorProcess>, LinuxSupervisorLaunchError> {
        if executor != self.executor_protection || (executor && self.worker.is_none()) {
            let primary = io::Error::new(
                io::ErrorKind::PermissionDenied,
                "executor requires protected birth and held worker; legacy materials forbidden",
            );
            // 此分支尚未调用clone；显式清除同一原预留，不能将无子进程的拒绝留下为永久占用。
            let cleanup = reservation.abort_before_birth(deadline);
            return Err(LinuxSupervisorLaunchError {
                error: match cleanup {
                    Ok(()) => primary,
                    Err(cleanup) => io::Error::other(
                        ChildError::io("executor birth admission", primary).with_cleanup(Err(
                            ChildError::io(
                                "original unborn reservation",
                                io::Error::other(cleanup),
                            ),
                        )),
                    ),
                },
                owner: None,
            });
        }
        let argv: Vec<_> = self
            .args
            .iter()
            .map(|arg| arg.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect();
        let environment: Vec<_> = self
            .environment
            .iter()
            .map(|arg| arg.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect();
        let slot = reservation
            .activate(deadline)
            .map_err(|error| LinuxSupervisorLaunchError {
                error: io::Error::other(error),
                owner: None,
            })?;
        // 两个完整处置槽均先于内核出生分配；ACTIVE 原锁跨越每个后续失败。
        let mut owner = Box::new(LinuxSupervisorProcess::before_birth(slot));
        let mut child_slot = Box::<LinuxAtomicChild>::new_uninit();
        let base = self.pipes.child_fds();
        let request = LinuxSupervisorAbi {
            fds: [
                base[0],
                base[1],
                base[2],
                base[3],
                base[4],
                self.extra[0].as_raw_fd(),
                self.extra[1].as_raw_fd(),
            ],
            uid: self.identity.0,
            gid: self.identity.1,
            argv: argv.as_ptr(),
            environment: environment.as_ptr(),
            group_count: self.groups.len() as u32,
            groups: self.groups.as_ptr(),
            worker_fd: self.worker.as_ref().map_or(0, AsRawFd::as_raw_fd),
            executor_protection: u32::from(self.executor_protection),
        };
        let mut mask = LinuxAtomicSignalMask::new();
        let mut pidfd = -1;
        if Instant::now() >= deadline {
            return Err(LinuxSupervisorLaunchError {
                error: io::Error::new(io::ErrorKind::TimedOut, "original birth deadline"),
                owner: Some(owner),
            });
        }
        let pid = unsafe {
            dg_linux_supervisor_spawn(&request, &mut pidfd, &mut mask.original, &mut mask.active)
        };
        if pid < 0 {
            // 未出生也返回原槽；只允许显式 abort_unborn，不自动改写失败记录。
            return Err(LinuxSupervisorLaunchError {
                error: io::Error::other(
                    ChildError::io(
                        "supervisor atomic birth",
                        io::Error::from_raw_os_error(-pid as i32),
                    )
                    .with_cleanup(mask.restore()),
                ),
                owner: Some(owner),
            });
        }
        // 到原 pidfd 采用之间无分配、回调、断言或格式化；child 永不返回 Rust 栈。
        let fd = unsafe { OwnedFd::from_raw_fd(pidfd) };
        child_slot.write(LinuxAtomicChild::adopt(
            LinuxAtomicExit::adopt(fd, pid as i32),
            self.pipes,
        ));
        owner.child = Some(unsafe { child_slot.assume_init() });
        owner.born = true;
        drop(self.extra);
        drop(self.worker);
        let startup = (|| -> io::Result<()> {
            mask.restore().map_err(io::Error::other)?;
            let peer = LinuxSupervisorPeer::from_host(pid as i32, self.identity.0, self.identity.1)
                .map_err(io::Error::other)?;
            // 模式由出生前受信宿主固定；错误不刷新原启动deadline或重建原控制端点。
            owner.controls = Some(
                if lifecycle {
                    LinuxControlReader::new_lifecycle(self.controls, peer, session)
                } else {
                    LinuxControlReader::new(self.controls, peer, session, deadline)
                }
                .map_err(io::Error::other)?,
            );
            if executor {
                owner.executor_materials = Some(
                    LinuxExecutorMaterials::sender(self.materials, session, deadline)
                        .map_err(io::Error::other)?,
                );
            } else {
                let peer =
                    LinuxSupervisorPeer::from_host(pid as i32, self.identity.0, self.identity.1)
                        .map_err(io::Error::other)?;
                owner.materials = Some(
                    LinuxSupervisorMaterials::new_sender(self.materials, peer, session, deadline)
                        .map_err(io::Error::other)?,
                );
            }
            let child = owner.child.as_mut().expect("original pidfd adopted");
            child.configure().map_err(io::Error::other)?;
            LinuxAtomicHandshake::run_supervisor(child, deadline, &mut || Ok::<(), ()>(())).map_err(
                |error| match error {
                    ChildSpawnError::Operation(error) => io::Error::other(error),
                    ChildSpawnError::Checkpoint { .. } => {
                        io::Error::new(io::ErrorKind::TimedOut, "original startup checkpoint")
                    }
                },
            )
        })();
        drop(self.image);
        match startup {
            Ok(()) => Ok(owner),
            Err(error) => Err(LinuxSupervisorLaunchError {
                error,
                owner: Some(owner),
            }),
        }
    }
}

fn duplicate(fd: i32) -> io::Result<OwnedFd> {
    let raw = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 10) };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

#[cfg(test)]
#[path = "linux_executor_launcher_tests.rs"]
mod tests;
