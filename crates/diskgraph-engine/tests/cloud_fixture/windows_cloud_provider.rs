//! 无云账户、无网络的真实 CFAPI 测试提供方；只管理独占临时根。
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use windows_sys::Win32::Foundation::STATUS_CLOUD_FILE_UNSUCCESSFUL;
use windows_sys::Win32::Storage::CloudFilters::{
    CF_CALLBACK_INFO, CF_CALLBACK_PARAMETERS, CF_CALLBACK_REGISTRATION,
    CF_CALLBACK_TYPE_FETCH_DATA, CF_CALLBACK_TYPE_NONE, CF_CONNECT_FLAG_REQUIRE_PROCESS_INFO,
    CF_CONNECTION_KEY, CF_CREATE_FLAG_STOP_ON_ERROR, CF_FS_METADATA, CF_HYDRATION_POLICY,
    CF_HYDRATION_POLICY_PARTIAL, CF_OPERATION_INFO, CF_OPERATION_PARAMETERS,
    CF_OPERATION_PARAMETERS_0, CF_OPERATION_PARAMETERS_0_0, CF_OPERATION_TYPE_TRANSFER_DATA,
    CF_PLACEHOLDER_CREATE_FLAG_MARK_IN_SYNC, CF_PLACEHOLDER_CREATE_INFO, CF_POPULATION_POLICY,
    CF_POPULATION_POLICY_ALWAYS_FULL, CF_SYNC_POLICIES, CF_SYNC_REGISTRATION, CfConnectSyncRoot,
    CfCreatePlaceholders, CfDisconnectSyncRoot, CfExecute, CfRegisterSyncRoot,
    CfUnregisterSyncRoot,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_NORMAL, FILE_BASIC_INFO};

// 此独立测试二进制仅有一个 provider 测试；回调无堆上下文，断开失败也不存在悬空指针。
static FETCHES: AtomicUsize = AtomicUsize::new(0);
static CALLBACK_FAILURE: AtomicI32 = AtomicI32::new(0);

/// 独占临时目录的原生占位文件提供方。来源：Microsoft CFAPI；无 Java 对应对象。
pub(crate) struct WindowsCloudProvider {
    root: PathBuf,
    wide_root: Vec<u16>,
    connection: Option<CF_CONNECTION_KEY>,
    registered: bool,
}

impl WindowsCloudProvider {
    /// 注册传入的独占空目录并连接只拒绝取数的提供方；返回 HRESULT 错误，不伪造平台支持。
    pub(crate) fn connect(root: &Path) -> Result<Self, String> {
        FETCHES.store(0, Ordering::SeqCst);
        CALLBACK_FAILURE.store(0, Ordering::SeqCst);
        let wide_root = wide(root.as_os_str());
        let name = wide(std::ffi::OsStr::new("DiskGraph isolated qualification"));
        let version = wide(std::ffi::OsStr::new("0.3.0"));
        let registration = CF_SYNC_REGISTRATION {
            StructSize: std::mem::size_of::<CF_SYNC_REGISTRATION>() as u32,
            ProviderName: name.as_ptr(),
            ProviderVersion: version.as_ptr(),
            ProviderId: windows_sys::core::GUID {
                data1: 0x831ebc41,
                data2: 0x2eae,
                data3: 0x4d29,
                data4: [0x81, 0xbe, 0x47, 0x15, 0x9c, 0xc0, 0x91, 0x62],
            },
            ..Default::default()
        };
        let policies = CF_SYNC_POLICIES {
            StructSize: std::mem::size_of::<CF_SYNC_POLICIES>() as u32,
            Hydration: CF_HYDRATION_POLICY {
                Primary: CF_HYDRATION_POLICY_PARTIAL,
                Modifier: 0,
            },
            Population: CF_POPULATION_POLICY {
                Primary: CF_POPULATION_POLICY_ALWAYS_FULL,
                Modifier: 0,
            },
            InSync: 0,
            HardLink: 0,
            PlaceholderManagement: 0,
        };
        check("register", unsafe {
            CfRegisterSyncRoot(wide_root.as_ptr(), &registration, &policies, 0)
        })?;
        let mut provider = Self {
            root: root.to_owned(),
            wide_root,
            connection: None,
            registered: true,
        };
        let callbacks = [
            CF_CALLBACK_REGISTRATION {
                Type: CF_CALLBACK_TYPE_FETCH_DATA,
                Callback: Some(reject_fetch),
            },
            CF_CALLBACK_REGISTRATION {
                Type: CF_CALLBACK_TYPE_NONE,
                Callback: None,
            },
        ];
        let mut connection = CF_CONNECTION_KEY::default();
        check("connect", unsafe {
            CfConnectSyncRoot(
                provider.wide_root.as_ptr(),
                callbacks.as_ptr(),
                std::ptr::null(),
                CF_CONNECT_FLAG_REQUIRE_PROCESS_INFO,
                &mut connection,
            )
        })?;
        provider.connection = Some(connection);
        Ok(provider)
    }

    /// 在本提供方独占根创建非空真实占位文件；返回路径，绝不提供正文或物化它。
    pub(crate) fn create_placeholder(&self) -> Result<PathBuf, String> {
        let name = wide(std::ffi::OsStr::new("dehydrated.bin"));
        let identity = b"diskgraph-isolated-cloud-file";
        let mut placeholder = CF_PLACEHOLDER_CREATE_INFO {
            RelativeFileName: name.as_ptr(),
            FsMetadata: CF_FS_METADATA {
                BasicInfo: FILE_BASIC_INFO {
                    FileAttributes: FILE_ATTRIBUTE_NORMAL,
                    ..Default::default()
                },
                FileSize: 4096,
            },
            FileIdentity: identity.as_ptr().cast(),
            FileIdentityLength: identity.len() as u32,
            Flags: CF_PLACEHOLDER_CREATE_FLAG_MARK_IN_SYNC,
            ..Default::default()
        };
        let mut processed = 0;
        check("create placeholder", unsafe {
            CfCreatePlaceholders(
                self.wide_root.as_ptr(),
                &mut placeholder,
                1,
                CF_CREATE_FLAG_STOP_ON_ERROR,
                &mut processed,
            )
        })?;
        check("placeholder result", placeholder.Result)?;
        if processed != 1 {
            return Err("actual placeholder was not processed".into());
        }
        Ok(self.root.join("dehydrated.bin"))
    }

    /// 返回本测试进程的实际 FETCH_DATA 数量；其他系统进程的扫描不冒充产品请求。
    pub(crate) fn fetches(&self) -> usize {
        FETCHES.load(Ordering::SeqCst)
    }

    /// 返回回调失败投影；零表示 CfExecute 已接收拒绝操作，不表示供给了正文。
    pub(crate) fn callback_failure(&self) -> i32 {
        CALLBACK_FAILURE.load(Ordering::SeqCst)
    }

    /// 先断开原连接再注销独占根；无参数，返回真实平台结果，失败保留状态以供析构重试。
    pub(crate) fn finish(&mut self) -> Result<(), String> {
        if let Some(connection) = self.connection {
            check("disconnect", unsafe { CfDisconnectSyncRoot(connection) })?;
            self.connection = None;
        }
        if self.registered {
            check("unregister", unsafe {
                CfUnregisterSyncRoot(self.wide_root.as_ptr())
            })?;
            self.registered = false;
        }
        Ok(())
    }
}

impl Drop for WindowsCloudProvider {
    fn drop(&mut self) {
        if let Err(error) = self.finish() {
            eprintln!("isolated cloud provider cleanup failed: {error}");
        }
    }
}

fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}
fn check(operation: &str, status: i32) -> Result<(), String> {
    if status < 0 {
        Err(format!("CFAPI {operation}: HRESULT {status:#010x}"))
    } else {
        Ok(())
    }
}

unsafe extern "system" fn reject_fetch(
    info: *const CF_CALLBACK_INFO,
    parameters: *const CF_CALLBACK_PARAMETERS,
) {
    let (Some(info), Some(parameters)) = (unsafe { info.as_ref() }, unsafe { parameters.as_ref() })
    else {
        return;
    };
    if unsafe { info.ProcessInfo.as_ref() }
        .is_some_and(|process| process.ProcessId == std::process::id())
    {
        FETCHES.fetch_add(1, Ordering::SeqCst);
    }
    let fetch = unsafe { parameters.Anonymous.FetchData };
    let operation = CF_OPERATION_INFO {
        StructSize: std::mem::size_of::<CF_OPERATION_INFO>() as u32,
        Type: CF_OPERATION_TYPE_TRANSFER_DATA,
        ConnectionKey: info.ConnectionKey,
        TransferKey: info.TransferKey,
        RequestKey: info.RequestKey,
        ..Default::default()
    };
    let mut transfer = CF_OPERATION_PARAMETERS {
        ParamSize: (std::mem::offset_of!(CF_OPERATION_PARAMETERS, Anonymous)
            + std::mem::size_of::<CF_OPERATION_PARAMETERS_0_0>()) as u32,
        Anonymous: CF_OPERATION_PARAMETERS_0 {
            TransferData: CF_OPERATION_PARAMETERS_0_0 {
                Flags: 0,
                CompletionStatus: STATUS_CLOUD_FILE_UNSUCCESSFUL,
                Buffer: std::ptr::null(),
                Offset: fetch.RequiredFileOffset,
                Length: fetch.RequiredLength,
            },
        },
    };
    // 无网络、无正文 buffer；失败完成真实请求，使普通读取正控不能卡住等待数据。
    let status = unsafe { CfExecute(&operation, &mut transfer) };
    if status < 0 {
        CALLBACK_FAILURE.store(status, Ordering::SeqCst);
    }
}
