use crate::{FileIdentity, WindowsObservationError, WindowsTreeAlignment};

/// 不截断的 Windows 卷、128 位文件身份与独立属性采样版本。
/// 来源：Windows FILE_ID_INFO/FILE_BASIC_INFO/FILE_STANDARD_INFO，DiskGraph 原生 Rust D31。
/// 时间值保留原始有符号 100ns 单位；不承诺文件系统精度、永久身份或原子内容快照。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowsFileObservation {
    pub volume: u64,
    pub file_id: [u8; 16],
    pub length: u64,
    pub creation_time: i64,
    pub last_write_time: i64,
    pub change_time: i64,
    pub attributes: u32,
    pub directory: bool,
    pub delete_pending: bool,
    pub capture_started_unix_ms: u64,
    pub capture_finished_unix_ms: u64,
    pub tree_alignment: WindowsTreeAlignment,
}

impl WindowsFileObservation {
    /// 持久化格式标签；与固定字节布局一起演进。
    pub const FORMAT_LABEL: &'static str = "windows_file_observation_v1";
    /// v1 完整记录的固定字节数。
    pub const ENCODED_LEN: usize = 80;

    /// 校验公开字段的语义。参数：无；返回：有效状态或明确字段错误。
    pub fn validate(&self) -> Result<(), WindowsObservationError> {
        if self.capture_finished_unix_ms < self.capture_started_unix_ms {
            return Err(WindowsObservationError::InvalidWindow);
        }
        if self.length > i64::MAX as u64 {
            return Err(WindowsObservationError::InvalidFileLength);
        }
        if self.directory != (self.attributes & 0x10 != 0) {
            return Err(WindowsObservationError::InconsistentDirectory);
        }
        Ok(())
    }

    /// 编码固定版本小端记录。参数：无；返回：完整 80 字节或校验错误。
    pub fn encode(&self) -> Result<[u8; Self::ENCODED_LEN], WindowsObservationError> {
        self.validate()?;
        let mut bytes = [0; Self::ENCODED_LEN];
        bytes[0] = 1;
        bytes[1..9].copy_from_slice(&self.volume.to_le_bytes());
        bytes[9..25].copy_from_slice(&self.file_id);
        bytes[25..33].copy_from_slice(&self.length.to_le_bytes());
        bytes[33..41].copy_from_slice(&self.creation_time.to_le_bytes());
        bytes[41..49].copy_from_slice(&self.last_write_time.to_le_bytes());
        bytes[49..57].copy_from_slice(&self.change_time.to_le_bytes());
        bytes[57..61].copy_from_slice(&self.attributes.to_le_bytes());
        bytes[61] = u8::from(self.directory);
        bytes[62] = u8::from(self.delete_pending);
        bytes[63..71].copy_from_slice(&self.capture_started_unix_ms.to_le_bytes());
        bytes[71..79].copy_from_slice(&self.capture_finished_unix_ms.to_le_bytes());
        bytes[79] = match self.tree_alignment {
            WindowsTreeAlignment::Matched => 0,
            WindowsTreeAlignment::Unverified => 1,
        };
        Ok(bytes)
    }

    /// 严格解码完整版本。参数：raw 为整条记录；返回：保留全部位的观测或协议错误。
    pub fn decode(raw: &[u8]) -> Result<Self, WindowsObservationError> {
        let bytes: &[u8; Self::ENCODED_LEN] = raw
            .try_into()
            .map_err(|_| WindowsObservationError::InvalidLength)?;
        if bytes[0] != 1 {
            return Err(WindowsObservationError::UnknownVersion);
        }
        let boolean = |value| match value {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(WindowsObservationError::InvalidTag),
        };
        // 长度已经固定；显式范围避免结构内存布局、对齐或宿主端序影响协议。
        let observation = Self {
            volume: u64::from_le_bytes(bytes[1..9].try_into().unwrap()),
            file_id: bytes[9..25].try_into().unwrap(),
            length: u64::from_le_bytes(bytes[25..33].try_into().unwrap()),
            creation_time: i64::from_le_bytes(bytes[33..41].try_into().unwrap()),
            last_write_time: i64::from_le_bytes(bytes[41..49].try_into().unwrap()),
            change_time: i64::from_le_bytes(bytes[49..57].try_into().unwrap()),
            attributes: u32::from_le_bytes(bytes[57..61].try_into().unwrap()),
            directory: boolean(bytes[61])?,
            delete_pending: boolean(bytes[62])?,
            capture_started_unix_ms: u64::from_le_bytes(bytes[63..71].try_into().unwrap()),
            capture_finished_unix_ms: u64::from_le_bytes(bytes[71..79].try_into().unwrap()),
            tree_alignment: match bytes[79] {
                0 => WindowsTreeAlignment::Matched,
                1 => WindowsTreeAlignment::Unverified,
                _ => return Err(WindowsObservationError::InvalidTag),
            },
        };
        observation.validate()?;
        Ok(observation)
    }

    /// 获取完整卷序号键。参数：无；返回：不依赖文件 ID 投影的固定十六进制卷键。
    pub fn volume_key(&self) -> String {
        format!("windows-volume-{:016x}", self.volume)
    }

    /// 保真投影到旧身份。参数：无；返回：字段有效且 ID 高 64 位为零时的旧对象，否则 None。
    pub fn legacy_identity(&self) -> Option<FileIdentity> {
        self.validate().ok()?;
        if self.file_id[8..].iter().any(|byte| *byte != 0) {
            return None;
        }
        Some(FileIdentity {
            volume_id: self.volume_key(),
            file_id: u64::from_le_bytes(self.file_id[..8].try_into().unwrap()),
        })
    }
}
