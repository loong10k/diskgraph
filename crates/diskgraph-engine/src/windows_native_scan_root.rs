use std::ffi::OsStr;
use std::fs::File;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use diskgraph_core::{
    BusinessError, DiskNode, NodeKind, ScanSettings, WindowsFileObservation, WindowsObservationGap,
    WindowsTreeAlignment,
};

use crate::EngineError;
use crate::scoped_content::ScopedContent;
use crate::windows_file_state::WindowsFileState;
use crate::windows_native_open::{open_child, open_child_utf16, open_drive};
use crate::windows_path_plan::WindowsPathPlan;
use crate::windows_scan_cost::WindowsScanCost;

/// 扫描期间保留 drive 到注册根的属性租约；来源：D31 与 NtCreateFile RootDirectory。
/// 它约束补充属性读取，不把上游路径线程池扫描或文件内容称为原子快照。
pub(crate) struct WindowsNativeScanRoot {
    path_plan: WindowsPathPlan,
    drive_root: PathBuf,
    components: Vec<Vec<u16>>,
    chain: Vec<File>,
    identities: Vec<WindowsFileState>,
    cost: Option<WindowsScanCost>,
    #[cfg(test)]
    root_captures: std::sync::atomic::AtomicUsize,
}

impl WindowsNativeScanRoot {
    /// 在路径扫描开始前固定注册根及全部祖先，允许注册根为 drive 根。
    /// 参数：root 为精确原生本地绝对路径，check 为期限/取消/实时授权/fence 检查。
    /// 返回：保留到发布结束的属性租约；根重解析、跨卷或检查失败直接返回错误。
    pub(crate) fn open(
        root: &Path,
        check: &dyn Fn() -> Result<(), EngineError>,
    ) -> Result<Self, EngineError> {
        let _guard = checked(check, ScopedContent::hydration_guard)??;
        let plan = checked(check, || WindowsPathPlan::for_root(root))??;
        let drive = checked(check, || open_drive(&plan.drive_root))??;
        let state = WindowsFileState::capture_checked(&drive, check)??;
        checked(check, || state.validate(true))??;
        let volume = state.volume;
        let mut chain = vec![drive];
        let mut identities = vec![state];
        for name in &plan.components {
            let parent = chain.last().expect("drive lease exists");
            let child = checked(check, || open_child(parent, name, true))??;
            let state = WindowsFileState::capture_checked(&child, check)??;
            checked(check, || state.validate(true))??;
            if state.volume != volume {
                return Err(BusinessError::Unsupported.into());
            }
            chain.push(child);
            identities.push(state);
        }
        let lease = Self {
            drive_root: plan.drive_root.clone(),
            // 只预编码固定名称；后续仍逐次重新打开并捕获完整根链身份。
            components: plan
                .components
                .iter()
                .map(|name| name.encode_wide().collect())
                .collect(),
            path_plan: plan,
            chain,
            identities,
            cost: (std::env::var_os("DISKGRAPH_SCAN_DIAGNOSTICS").as_deref()
                == Some(std::ffi::OsStr::new("1")))
            .then(WindowsScanCost::default),
            #[cfg(test)]
            root_captures: std::sync::atomic::AtomicUsize::new(0),
        };
        lease.validate_held_root(check)?;
        Ok(lease)
    }

    /// 从固定根读取一个对象的属性，单次保留 O(depth) 临时父链，不申请正文权限。
    /// 参数：path/node 为精确定位和旧树值，self_modified 是旧秒级信息且不证明内容；
    /// settings 限定可比较的 apparent/non-dedup 尺寸，check 的失败必须原样传播。
    /// 返回：完整原生观测或固定 Gap；根失效与 check 错误直接失败，不转成 unknown。
    pub(crate) fn observe(
        &self,
        path: &Path,
        node: &DiskNode,
        self_modified: Option<i64>,
        settings: &ScanSettings,
        check: &dyn Fn() -> Result<(), EngineError>,
    ) -> Result<
        (
            Option<WindowsFileObservation>,
            Option<WindowsObservationGap>,
        ),
        EngineError,
    > {
        let guard = checked(check, ScopedContent::hydration_guard)?;
        let _guard = match guard {
            Ok(guard) => guard,
            Err(error) => return Ok(gap(error)),
        };
        self.validate_held_root(check)?;
        let names = checked(check, || self.path_plan.relative_path(path))?;
        let names = match names {
            Ok(names) => names,
            Err(error) => return Ok(gap(error)),
        };
        let root = self.chain.last().expect("registered root lease exists");
        let volume = self.identities.last().expect("root identity exists").volume;
        let mut parents = Vec::new();
        for name in names.iter().take(names.len().saturating_sub(1)) {
            let parent = parents.last().unwrap_or(root);
            let child = checked(check, || open_child(parent, name, true))?;
            let child = match child {
                Ok(child) => child,
                Err(error) => return Ok(gap(error)),
            };
            let state = WindowsFileState::capture_checked(&child, check)?;
            let state = match state {
                Ok(state) => state,
                Err(error) => return Ok(gap(error)),
            };
            let valid = checked(check, || state.validate(true))?;
            if let Err(error) = valid {
                return Ok(gap(error));
            }
            if state.volume != volume {
                return Ok((None, Some(WindowsObservationGap::Unsupported)));
            }
            parents.push(child);
        }
        let leaf = if let Some(name) = names.last() {
            match checked(check, || {
                open_child(parents.last().unwrap_or(root), name, false)
            })? {
                Ok(leaf) => Some(leaf),
                Err(error) => return Ok(gap(error)),
            }
        } else {
            None
        };
        let leaf = leaf.as_ref().unwrap_or(root);
        let started = match checked(check, capture_time)? {
            Ok(started) => started,
            Err(error) => return Ok(gap(error)),
        };
        let before = match WindowsFileState::capture_checked(leaf, check)? {
            Ok(state) => state,
            Err(error) => return Ok(gap(error)),
        };
        #[cfg(test)]
        crate::windows_native_scan_tests::between_captures();
        let after = match WindowsFileState::capture_checked(leaf, check)? {
            Ok(state) => state,
            Err(error) => return Ok(gap(error)),
        };
        let finished = match checked(check, capture_time)? {
            Ok(finished) => finished,
            Err(error) => return Ok(gap(error)),
        };
        if started > finished {
            return Ok((None, Some(WindowsObservationGap::CaptureFailed)));
        }
        if before != after {
            return Ok((None, Some(WindowsObservationGap::Changed)));
        }
        if after.volume != volume {
            return Ok((None, Some(WindowsObservationGap::Unsupported)));
        }
        // 秒级 mtime 不用于补出树/正文一致性；原生 ticks 独立保存在新观测中。
        let _ = self_modified;
        let alignment = match alignment(&after, node, settings) {
            Ok(alignment) => alignment,
            Err(gap) => return Ok((None, Some(gap))),
        };
        let observation = after.observation(started, finished, alignment);
        if observation.validate().is_err() {
            return Ok((None, Some(WindowsObservationGap::CaptureFailed)));
        }
        self.validate_held_root(check)?;
        check()?;
        Ok((Some(observation), None))
    }

    /// 发布前重新验证保留根链及当前名称绑定，不比较允许变化的目录修改时间。
    /// 参数：check 为任务原始期限、取消、实时授权与 fencing 检查。
    /// 返回：根链与当前绑定的身份/卷/目录/非重解析/未删除保持时成功，否则拒绝发布。
    pub(crate) fn validate_root(
        &self,
        check: &dyn Fn() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        let _guard = checked(check, ScopedContent::hydration_guard)??;
        self.validate_held_root(check)
    }

    /// 从保留的根目录打开单个末叶属性并核根链；来源：PF-06 原镜像名称绑定。
    /// 参数：name 是单个原生名称，check 是同次请求检查；返回：非链接、非占位同卷文件。
    /// 此接口不申请正文权限，也不通过完整名称追随替换后的父目录。
    pub(crate) fn open_leaf_attributes(
        &self,
        name: &OsStr,
        check: &dyn Fn() -> Result<(), EngineError>,
    ) -> Result<File, EngineError> {
        let _guard = checked(check, ScopedContent::hydration_guard)??;
        self.validate_held_root(check)?;
        let parent = self.chain.last().expect("registered root lease exists");
        let file = checked(check, || open_child(parent, name, false))??;
        let state = WindowsFileState::capture_checked(&file, check)??;
        checked(check, || state.validate(false))??;
        if state.placeholder()
            || state.volume != self.identities.last().expect("root identity exists").volume
        {
            return Err(BusinessError::Unsupported.into());
        }
        self.validate_held_root(check)?;
        check()?;
        Ok(file)
    }

    /// 参数：nodes为当前已计费节点数；返回：无，显式诊断时输出有限纯数值。
    /// 累计根检查包括租约出生至staging完成，不代表单纯内核调用或业务总成本。
    pub(crate) fn emit_cost_diagnostic(&self, nodes: u64) {
        if let Some(cost) = &self.cost {
            let (calls, total_ms) = cost.snapshot();
            eprintln!(
                "diskgraph: scan_cost=windows_root_validation nodes={nodes} total_ms={total_ms}"
            );
            eprintln!(
                "diskgraph: scan_root_validation calls={calls} chain_handles={} total_ms={total_ms}",
                self.chain.len()
            );
        }
    }

    fn validate_held_root(
        &self,
        check: &dyn Fn() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        match &self.cost {
            Some(cost) => cost.measure(|| self.validate_held_root_inner(check)),
            None => self.validate_held_root_inner(check),
        }
    }

    fn validate_held_root_inner(
        &self,
        check: &dyn Fn() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        // 属性句柄不冻结目录名称。原对象即使被重命名，其句柄身份也可能不变。
        // 重新核对 drive 与保留父句柄下每个名称，绝不从注册根的完整路径追随新对象。
        // 当前绑定必须匹配原卷、完整ID、创建时间及安全属性；保留句柄不会更换对象，
        // 因而无需在这轮新鲜捕获之前再读取同一保留对象。不能省略名称重新打开。
        let drive = checked(check, || open_drive(&self.drive_root))?
            .map_err(|_| EngineError::Business(BusinessError::Conflict))?;
        self.validate_binding(&drive, &self.identities[0], check)?;
        for (index, name) in self.components.iter().enumerate() {
            let current = checked(check, || open_child_utf16(&self.chain[index], name, true))?
                .map_err(|_| EngineError::Business(BusinessError::Conflict))?;
            self.validate_binding(&current, &self.identities[index + 1], check)?;
        }
        check()
    }

    fn validate_binding(
        &self,
        file: &File,
        identity: &WindowsFileState,
        check: &dyn Fn() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        // 外层检查错误原样传播；无法确认当前 namespace 绑定则保守拒绝该根。
        let current = self
            .capture_root(file, check)?
            .map_err(|_| EngineError::Business(BusinessError::Conflict))?;
        if !identity.matches_scan_root(&current) || current.validate(true).is_err() {
            return Err(BusinessError::Conflict.into());
        }
        check()
    }

    fn capture_root(
        &self,
        file: &File,
        check: &dyn Fn() -> Result<(), EngineError>,
    ) -> Result<Result<WindowsFileState, EngineError>, EngineError> {
        #[cfg(test)]
        self.root_captures
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        WindowsFileState::capture_checked(file, check)
    }
}

fn alignment(
    state: &WindowsFileState,
    node: &DiskNode,
    settings: &ScanSettings,
) -> Result<WindowsTreeAlignment, WindowsObservationGap> {
    let kind = if state.reparse() {
        NodeKind::Symlink
    } else if state.directory {
        NodeKind::Directory
    } else {
        NodeKind::File
    };
    if node.kind != kind {
        return Err(WindowsObservationGap::TreeMismatch);
    }
    let comparable_size = kind == NodeKind::File
        && settings.apparent_size
        && !settings.dedup_hardlinks
        && node.size_known
        && !node.read_error;
    if comparable_size && node.direct_bytes != state.len {
        return Err(WindowsObservationGap::TreeMismatch);
    }
    let Some(old) = &node.file_identity else {
        return Ok(WindowsTreeAlignment::Unverified);
    };
    if old.volume_id != format!("windows-volume-{:016x}", state.volume) {
        return Err(WindowsObservationGap::TreeMismatch);
    }
    let Some(current) = state.legacy_identity() else {
        return Ok(WindowsTreeAlignment::Unverified);
    };
    if old != &current {
        return Err(WindowsObservationGap::TreeMismatch);
    }
    Ok(if comparable_size {
        WindowsTreeAlignment::Matched
    } else {
        WindowsTreeAlignment::Unverified
    })
}

// 外层仅传播调用方检查错误；内层 native 失败才能转换成固定观测 Gap。
fn checked<T>(
    check: &dyn Fn() -> Result<(), EngineError>,
    native: impl FnOnce() -> Result<T, EngineError>,
) -> Result<Result<T, EngineError>, EngineError> {
    check()?;
    let result = native();
    check()?;
    Ok(result)
}

fn capture_time() -> Result<u64, EngineError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .ok_or_else(|| BusinessError::InternalError.into())
}

fn gap(
    error: EngineError,
) -> (
    Option<WindowsFileObservation>,
    Option<WindowsObservationGap>,
) {
    let gap = match error {
        EngineError::Business(
            BusinessError::Unsupported
            | BusinessError::InvalidArgument
            | BusinessError::PermissionDenied,
        ) => WindowsObservationGap::Unsupported,
        EngineError::Business(BusinessError::Conflict) => WindowsObservationGap::Changed,
        _ => WindowsObservationGap::CaptureFailed,
    };
    (None, Some(gap))
}

#[cfg(test)]
mod validation_cost_tests {
    use super::WindowsNativeScanRoot;
    use std::sync::atomic::Ordering;

    #[test]
    fn current_namespace_validation_captures_each_bound_object_once() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let lease = WindowsNativeScanRoot::open(&root, &|| Ok(())).unwrap();
        lease.root_captures.store(0, Ordering::Relaxed);
        lease.validate_root(&|| Ok(())).unwrap();
        assert_eq!(
            lease.root_captures.load(Ordering::Relaxed),
            lease.chain.len(),
            "fresh namespace binding must not recapture the same held objects first"
        );
    }
}
