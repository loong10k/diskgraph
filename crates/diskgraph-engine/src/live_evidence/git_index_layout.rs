use super::git_metadata_budget::GitMetadataBudget;
use super::probe_budget::ProbeBudget;
use sha1::{Digest, Sha1};
use sha2::Sha256;

/// Git index v2/v3/v4 的有界结构预检结果；来源：Git index-format 的 DIRC/entry/extension 布局。
/// 原始 index 字节仍由调用方原样复制；预检不改 stat、stage、OID 或缓存扩展。
pub(super) struct GitIndexLayout {
    pub(super) version: u32,
    pub(super) entry_count: u32,
    pub(super) has_fsmonitor: bool,
    pub(super) has_untracked_cache: bool,
}

impl GitIndexLayout {
    /// 预检原始 index 的边界、路径、特殊模式及扩展，不重新编码源文件。
    /// 参数：bytes 为已按元数据预算捕获的完整文件，oid_len 为对象 ID 字节数，budget/probe 为期限与取消。
    /// 返回：可进入私有索引构造的布局；不支持或损坏的结构明确拒绝。
    pub(super) fn parse(
        bytes: &[u8],
        oid_len: usize,
        budget: &mut GitMetadataBudget,
        probe: &mut ProbeBudget,
    ) -> Result<Self, String> {
        if !matches!(oid_len, 20 | 32) || bytes.len() < 12 + oid_len || &bytes[..4] != b"DIRC" {
            return Err("unsupported git index header".into());
        }
        let version = be32(bytes, 4)?;
        if !matches!(version, 2..=4) {
            return Err("unsupported git index version".into());
        }
        let entry_count = be32(bytes, 8)?;
        let end = bytes.len() - oid_len;
        let minimum = 42usize
            .checked_add(oid_len)
            .ok_or("invalid git index size")?;
        if entry_count as usize > (end - 12) / (minimum + 1) {
            return Err("invalid git index entry count".into());
        }
        let mut cursor = 12usize;
        let mut previous = Vec::<u8>::new();
        let mut previous_stage = 0u16;
        for index in 0..entry_count {
            budget.check(probe)?;
            let start = cursor;
            let fixed_end = cursor
                .checked_add(minimum)
                .ok_or("invalid git index entry")?;
            let fixed = bytes
                .get(cursor..fixed_end)
                .filter(|_| fixed_end <= end)
                .ok_or("truncated git index entry")?;
            let mode = u32::from_be_bytes(fixed[24..28].try_into().expect("fixed header"));
            if matches!(mode, 0o040000 | 0o160000) {
                return Err("unsupported git sparse directory or gitlink".into());
            }
            if !matches!(mode, 0o100644 | 0o100755 | 0o120000) {
                return Err("unsupported git index mode".into());
            }
            let flags = u16::from_be_bytes(
                fixed[40 + oid_len..42 + oid_len]
                    .try_into()
                    .expect("fixed flags"),
            );
            // bit15 是 Git 的 assume-valid；原样复制到私有 index 才能保持源仓库的状态语义。
            let stage = (flags >> 12) & 3;
            cursor = fixed_end;
            if flags & 0x4000 != 0 {
                if version == 2 {
                    return Err("invalid git index extended flags".into());
                }
                let extended = bytes
                    .get(cursor..cursor + 2)
                    .filter(|_| cursor + 2 <= end)
                    .ok_or("truncated git index extended flags")?;
                let value = u16::from_be_bytes(extended.try_into().expect("extended flags"));
                if value & !0x6000 != 0 {
                    return Err("unsupported git index extended flags".into());
                }
                cursor += 2;
            }
            let path = if version == 4 {
                let stripped = decode_varint(bytes, &mut cursor, end)?;
                if stripped > previous.len() {
                    return Err("invalid git index v4 path prefix".into());
                }
                let suffix = nul_field(bytes, &mut cursor, end)?;
                let mut path = previous[..previous.len() - stripped].to_vec();
                path.extend_from_slice(suffix);
                path
            } else {
                let path = nul_field(bytes, &mut cursor, end)?.to_vec();
                let padded = cursor
                    .checked_add((8 - ((cursor - start) % 8)) % 8)
                    .ok_or("invalid git index padding")?;
                if padded > end || bytes[cursor..padded].iter().any(|byte| *byte != 0) {
                    return Err("invalid git index padding".into());
                }
                cursor = padded;
                path
            };
            validate_path(&path)?;
            if flags & 0x0fff != 0x0fff && usize::from(flags & 0x0fff) != path.len() {
                return Err("invalid git index path length".into());
            }
            if index > 0
                && (path.as_slice() < previous.as_slice()
                    || (path == previous && stage <= previous_stage))
            {
                return Err("invalid git index entry order".into());
            }
            previous = path;
            previous_stage = stage;
        }
        let mut has_fsmonitor = false;
        let mut has_untracked_cache = false;
        while cursor < end {
            budget.check(probe)?;
            let header = bytes
                .get(cursor..cursor + 8)
                .filter(|_| cursor + 8 <= end)
                .ok_or("truncated git index extension")?;
            let signature = &header[..4];
            let len =
                u32::from_be_bytes(header[4..8].try_into().expect("extension length")) as usize;
            cursor = cursor
                .checked_add(8)
                .and_then(|value| value.checked_add(len))
                .filter(|value| *value <= end)
                .ok_or("invalid git index extension length")?;
            match signature {
                b"TREE" | b"REUC" | b"EOIE" | b"IEOT" => {}
                b"FSMN" => has_fsmonitor = true,
                b"UNTR" => has_untracked_cache = true,
                b"link" | b"sdir" => return Err("unsupported git index extension".into()),
                _ => return Err("unsupported git index extension".into()),
            }
        }
        if cursor != end {
            return Err("invalid git index trailing bytes".into());
        }
        budget.check(probe)?;
        // Git status/ls-files 可接受已损坏的 trailer，不能将其退出码当作 index 验真。
        let checksum = if oid_len == 20 {
            Sha1::digest(&bytes[..end]).to_vec()
        } else {
            Sha256::digest(&bytes[..end]).to_vec()
        };
        budget.check(probe)?;
        if checksum != bytes[end..] {
            return Err("invalid git index checksum".into());
        }
        Ok(Self {
            version,
            entry_count,
            has_fsmonitor,
            has_untracked_cache,
        })
    }
}

fn be32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    Ok(u32::from_be_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or("truncated git index header")?
            .try_into()
            .expect("four bytes"),
    ))
}

fn nul_field<'a>(bytes: &'a [u8], cursor: &mut usize, end: usize) -> Result<&'a [u8], String> {
    let field = bytes.get(*cursor..end).ok_or("invalid git index field")?;
    let size = field
        .iter()
        .position(|byte| *byte == 0)
        .ok_or("unterminated git index path")?;
    let result = &field[..size];
    *cursor = cursor
        .checked_add(size + 1)
        .ok_or("invalid git index field")?;
    Ok(result)
}

fn decode_varint(bytes: &[u8], cursor: &mut usize, end: usize) -> Result<usize, String> {
    let mut value = 0usize;
    for _ in 0..10 {
        let byte = *bytes
            .get(*cursor)
            .filter(|_| *cursor < end)
            .ok_or("truncated git index v4 path")?;
        *cursor += 1;
        value = value
            .checked_add((byte & 0x7f) as usize)
            .ok_or("invalid git index v4 path")?;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        value = value
            .checked_add(1)
            .and_then(|next| next.checked_mul(128))
            .ok_or("invalid git index v4 path")?;
    }
    Err("invalid git index v4 path".into())
}

fn validate_path(path: &[u8]) -> Result<(), String> {
    if path.is_empty()
        || path[0] == b'/'
        || path.last() == Some(&b'/')
        || path.split(|byte| *byte == b'/').any(|component| {
            component.is_empty()
                || component == b"."
                || component == b".."
                || component == b".git"
                || (cfg!(windows) && component.eq_ignore_ascii_case(b".git"))
        })
        || (cfg!(windows) && path.contains(&b'\\'))
    {
        return Err("unsupported git index path".into());
    }
    Ok(())
}
