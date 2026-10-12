use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io;

/// 不可变本地只读查询描述，只生成固定CLI参数而不授权资源。来源：PF-06，无Java对等对象。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerQuery {
    command: String,
    subcommand: Option<String>,
    options: Vec<(String, String)>,
    switches: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    native_root: Option<diskgraph_core::Locator>,
}

impl BrokerQuery {
    /// 参数：命令、子命令、值参数和布尔开关；返回：完整验证的固定查询或出生前拒绝。
    pub fn new(
        command: &str,
        subcommand: Option<&str>,
        options: Vec<(&str, String)>,
        switches: Vec<&str>,
    ) -> io::Result<Self> {
        let query = Self {
            command: command.to_owned(),
            subcommand: subcommand.map(str::to_owned),
            options: options
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value))
                .collect(),
            switches: switches.into_iter().map(str::to_owned).collect(),
            native_root: None,
        };
        query.validate()?;
        Ok(query)
    }

    /// 参数：无；返回：验证结果。反序列化后的请求必须在认领容量、创建进程前调用。
    pub fn validate(&self) -> io::Result<()> {
        let (required, optional, switches): (&[&str], &[&str], &[&str]) =
            match (self.command.as_str(), self.subcommand.as_deref()) {
                ("doctor", None) | ("scope", Some("list")) => (&[], &[], &[]),
                ("scope", Some("show")) | ("node", None) => (&["scope"], &[], &[]),
                ("scope", Some("add")) => (&[], &[], &[]),
                ("index" | "sync", None) => (&["scope"], &[], &["wait"]),
                ("status", None) => (&["job"], &[], &[]),
                ("snapshots", None) => (&[], &["scope", "limit", "offset"], &[]),
                ("tree", None) => (
                    &["scope"],
                    &["revision", "depth", "min-bytes"],
                    &["anonymize"],
                ),
                ("children", None) => (
                    &["scope"],
                    &["parent-id", "limit", "offset", "min-bytes"],
                    &["unknown-only"],
                ),
                ("top", None) => (&["scope"], &["parent-id", "limit"], &[]),
                ("search", None) => (&["scope", "pattern"], &["offset", "limit", "cursor"], &[]),
                ("explore", None) => (&["scope"], &["node-id", "max-depth", "max-nodes"], &[]),
                ("growth", None) => (&["scope", "before", "after"], &["path"], &[]),
                ("changes", None) => (&["scope", "before", "after"], &[], &[]),
                ("explain", None) => (
                    &["scope", "revision", "entity"],
                    &["limit", "after-edge"],
                    &[],
                ),
                ("related", None) => (
                    &["scope", "revision", "entity"],
                    &["relation", "limit", "after-edge"],
                    &["outgoing"],
                ),
                ("impact", None) => (
                    &["scope", "revision", "entity"],
                    &["max-depth", "max-edges"],
                    &[],
                ),
                ("candidates", None) => (&["scope"], &["target-bytes"], &[]),
                _ => return Err(invalid()),
            };
        let scope_add = self.command == "scope" && self.subcommand.as_deref() == Some("add");
        match self.native_root.as_ref() {
            Some(root) if scope_add && root.raw_b64.len() <= 4096 && root.display.len() <= 2048 => {
                let path = root.to_native_path().map_err(|_| invalid())?;
                if !path.is_absolute()
                    || path.as_os_str().as_encoded_bytes().len() > 2048
                    || path.as_os_str().as_encoded_bytes().contains(&0)
                {
                    return Err(invalid());
                }
            }
            None if !scope_add => {}
            _ => return Err(invalid()),
        }
        if self.options.len() > 16 || self.switches.len() > 4 {
            return Err(invalid());
        }
        let mut seen = HashSet::new();
        for (key, value) in &self.options {
            if !(required.contains(&key.as_str()) || optional.contains(&key.as_str()))
                || !seen.insert(key.as_str())
                || value.len() > 2048
                || value.contains('\0')
            {
                return Err(invalid());
            }
            match key.as_str() {
                "depth" | "max-depth" | "max-nodes" | "max-edges" => {
                    value.parse::<usize>().map_err(|_| invalid())?;
                }
                "parent-id" | "node-id" | "limit" | "offset" | "min-bytes" | "target-bytes" => {
                    value.parse::<u64>().map_err(|_| invalid())?;
                }
                "pattern" | "path" => {}
                _ if value.is_empty() => return Err(invalid()),
                _ => {}
            }
        }
        if required.iter().any(|key| !seen.contains(key)) {
            return Err(invalid());
        }
        for key in &self.switches {
            if !switches.contains(&key.as_str()) || !seen.insert(key.as_str()) {
                return Err(invalid());
            }
        }
        Ok(())
    }

    /// 参数：原绝对目录路径；返回：无损scope注册描述，不读取文件或签发资源权限。
    pub fn for_native_root(root: &std::path::Path) -> io::Result<Self> {
        let query = Self {
            command: "scope".into(),
            subcommand: Some("add".into()),
            options: Vec::new(),
            switches: Vec::new(),
            native_root: Some(diskgraph_core::Locator::from_native_path(root)),
        };
        query.validate()?;
        Ok(query)
    }

    /// 参数：无；返回：是否必须使用真实客户executor，旧service身份入口不得分发。
    pub fn requires_customer_executor(&self) -> bool {
        matches!(self.command.as_str(), "index" | "sync")
            || (self.command == "scope" && self.subcommand.as_deref() == Some("add"))
    }

    /// 参数：无；返回：原OS编码参数，不以display替代注册目标，值仍与固定开关为同一参数。
    pub fn native_arguments(&self) -> io::Result<Vec<std::ffi::OsString>> {
        self.validate()?;
        let mut result = vec![std::ffi::OsString::from(&self.command)];
        result.extend(self.subcommand.iter().map(std::ffi::OsString::from));
        result.extend(
            self.options
                .iter()
                .map(|(key, value)| std::ffi::OsString::from(format!("--{key}={value}"))),
        );
        result.extend(
            self.switches
                .iter()
                .map(|key| std::ffi::OsString::from(format!("--{key}"))),
        );
        if let Some(root) = &self.native_root {
            let mut argument = std::ffi::OsString::from("--root=");
            argument.push(root.to_native_path().map_err(|_| invalid())?);
            result.push(argument);
        }
        Ok(result)
    }

    /// 参数：无；返回：旧UTF-8参数兼容入口；非UTF-8目标明确拒绝，使用native_arguments保留编码。
    pub fn arguments(&self) -> io::Result<Vec<String>> {
        self.native_arguments()?
            .into_iter()
            .map(|argument| argument.into_string().map_err(|_| invalid()))
            .collect()
    }
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "unsupported fixed read-only query",
    )
}
