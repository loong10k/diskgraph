use crate::{BusinessError, JobAuthorityOrigin, Permission, PrincipalId};
use serde::{Deserialize, Deserializer, Serialize};

/// 保存服务器已验证的不可变任务身份与能力上限，不保存 bearer 或签名秘密。
/// 来源：DiskGraph 原生 Rust SC-06 持久请求授权设计，无 Java 对应对象。
/// 构造函数是可信 Rust 适配器边界，远程参数不得直接反序列化为此对象。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct JobRequestAuthority {
    origin: JobAuthorityOrigin,
    principal: PrincipalId,
    issuer: Option<String>,
    transport: String,
    capability_ceiling: Option<Vec<Permission>>,
    expires_at_unix_seconds: Option<u64>,
}

impl JobRequestAuthority {
    /// 构造显式可信本地请求。参数：principal 为实际本地主体，transport 为可信适配器名称。
    /// 返回：不可变上下文或非法身份/传输名称；不将缺失远程字段解释为本地请求。
    pub fn trusted_local(
        principal: PrincipalId,
        transport: impl Into<String>,
    ) -> Result<Self, BusinessError> {
        let authority = Self {
            origin: JobAuthorityOrigin::TrustedLocal,
            principal,
            issuer: None,
            transport: transport.into(),
            capability_ceiling: None,
            expires_at_unix_seconds: None,
        };
        authority.validate_structure()?;
        Ok(authority)
    }

    /// 构造实际认证后的远程请求。参数：身份、签发方、传输、原 token 能力与原始 Unix 秒到期值。
    /// 返回：规范化不可变上下文或格式错误；到期值不从新的时钟、凭据或租约推算。
    pub fn authenticated_remote(
        principal: PrincipalId,
        issuer: impl Into<String>,
        transport: impl Into<String>,
        mut capabilities: Vec<Permission>,
        expires_at_unix_seconds: u64,
    ) -> Result<Self, BusinessError> {
        if capabilities.len() > 64 {
            return Err(BusinessError::InvalidArgument);
        }
        capabilities.sort_by_key(|permission| permission.wire_name());
        capabilities.dedup();
        let authority = Self {
            origin: JobAuthorityOrigin::AuthenticatedRemote,
            principal,
            issuer: Some(issuer.into()),
            transport: transport.into(),
            capability_ceiling: Some(capabilities),
            expires_at_unix_seconds: Some(expires_at_unix_seconds),
        };
        authority.validate_structure()?;
        Ok(authority)
    }

    fn validate_structure(&self) -> Result<(), BusinessError> {
        PrincipalId::new(self.principal.as_str()).map_err(|_| BusinessError::InvalidArgument)?;
        if self.transport.is_empty() || self.transport.len() > 64 {
            return Err(BusinessError::InvalidArgument);
        }
        match self.origin {
            JobAuthorityOrigin::TrustedLocal
                if self.issuer.is_none()
                    && self.capability_ceiling.is_none()
                    && self.expires_at_unix_seconds.is_none() =>
            {
                Ok(())
            }
            JobAuthorityOrigin::AuthenticatedRemote
                if self
                    .issuer
                    .as_ref()
                    .is_some_and(|issuer| !issuer.is_empty() && issuer.len() <= 4096)
                    && self.capability_ceiling.is_some()
                    && self.expires_at_unix_seconds.is_some() =>
            {
                Ok(())
            }
            _ => Err(BusinessError::InvalidArgument),
        }
    }

    /// 检查原绝对认证有效期。参数：now_unix_seconds 为当前真实 Unix 秒。
    /// 返回：尚有效为 ()；恰好到期也拒绝，不应用 JWT admission 的时钟宽限。
    pub fn validate_at(&self, now_unix_seconds: u64) -> Result<(), BusinessError> {
        // 字段私有且反序列化经过构造校验；热路径只查原 exp，不重复分配主体字符串。
        if self
            .expires_at_unix_seconds
            .is_some_and(|expiry| now_unix_seconds >= expiry)
        {
            return Err(BusinessError::PermissionDenied);
        }
        Ok(())
    }

    /// 检查请求能力上限。参数：permission 为精确权限，now 为当前 Unix 秒。
    /// 返回：有效且请求上限包含该权限；实时数据库授权仍由调用方求交集。
    pub fn allows(&self, permission: &Permission, now_unix_seconds: u64) -> bool {
        self.validate_at(now_unix_seconds).is_ok()
            && self
                .capability_ceiling
                .as_ref()
                .is_none_or(|capabilities| capabilities.contains(permission))
    }

    /// 读取实际来源。参数：无；返回：显式来源，缺记录需由 Store 单独处理。
    pub fn origin(&self) -> JobAuthorityOrigin {
        self.origin
    }
    /// 读取真实主体。参数：无；返回：原请求主体的借用引用。
    pub fn principal(&self) -> &PrincipalId {
        &self.principal
    }
    /// 读取认证签发方。参数：无；返回：远程签发方或可信本地的 None。
    pub fn issuer(&self) -> Option<&str> {
        self.issuer.as_deref()
    }
    /// 读取传输审计名称。参数：无；返回：名称，不依据该字符串授予本地信任。
    pub fn transport(&self) -> &str {
        &self.transport
    }
    /// 读取不可变 token 能力上限。参数：无；返回：远程集合，None 仅为显式可信本地。
    pub fn capabilities(&self) -> Option<&[Permission]> {
        self.capability_ceiling.as_deref()
    }
    /// 读取原始到期值。参数：无；返回：原 token Unix 秒，可信本地为 None。
    pub fn expires_at_unix_seconds(&self) -> Option<u64> {
        self.expires_at_unix_seconds
    }
}

impl<'de> Deserialize<'de> for JobRequestAuthority {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let decode = || -> Result<Self, BusinessError> {
            let object = value.as_object().ok_or(BusinessError::InvalidArgument)?;
            let keys = [
                "origin",
                "principal",
                "issuer",
                "transport",
                "capability_ceiling",
                "expires_at_unix_seconds",
            ];
            if object.len() != keys.len() || keys.iter().any(|key| !object.contains_key(*key)) {
                return Err(BusinessError::InvalidArgument);
            }
            let origin: JobAuthorityOrigin = serde_json::from_value(value["origin"].clone())
                .map_err(|_| BusinessError::InvalidArgument)?;
            let principal = PrincipalId::new(
                value["principal"]
                    .as_str()
                    .ok_or(BusinessError::InvalidArgument)?,
            )
            .map_err(|_| BusinessError::InvalidArgument)?;
            let transport = value["transport"]
                .as_str()
                .ok_or(BusinessError::InvalidArgument)?;
            match origin {
                JobAuthorityOrigin::TrustedLocal => {
                    if !value["issuer"].is_null()
                        || !value["capability_ceiling"].is_null()
                        || !value["expires_at_unix_seconds"].is_null()
                    {
                        return Err(BusinessError::InvalidArgument);
                    }
                    Self::trusted_local(principal, transport)
                }
                JobAuthorityOrigin::AuthenticatedRemote => {
                    let issuer = value["issuer"]
                        .as_str()
                        .ok_or(BusinessError::InvalidArgument)?;
                    let capabilities = serde_json::from_value(value["capability_ceiling"].clone())
                        .map_err(|_| BusinessError::InvalidArgument)?;
                    let expiry = value["expires_at_unix_seconds"]
                        .as_u64()
                        .ok_or(BusinessError::InvalidArgument)?;
                    Self::authenticated_remote(principal, issuer, transport, capabilities, expiry)
                }
            }
        };
        decode().map_err(serde::de::Error::custom)
    }
}
