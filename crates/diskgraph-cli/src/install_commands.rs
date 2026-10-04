//! CLI install_commands 的真实职责实现。
use crate::install_command::InstallCommand;
use diskgraph_core::Envelope;
use diskgraph_engine::EngineError;

/// 保留 install_tool 的原生业务职责与错误语义。来源：DiskGraph CLI main::install_tool。
/// 参数：与原入口的 install_tool 请求及执行依赖相同。返回：原业务结果或真实执行错误。
pub(crate) fn install_tool(action: &InstallCommand) -> Result<(), EngineError> {
    use diskgraph_mcp::install::{Registration, add, remove, require_client, show};

    let outcome = match action {
        InstallCommand::Add {
            client,
            config,
            apply_config,
        } => {
            require_client(client).map_err(|error| {
                EngineError::Store(diskgraph_store::StoreError::InvalidGraph(error.to_string()))
            })?;
            let registration = Registration::stdio(
                std::env::current_exe()
                    .map_err(|error| EngineError::Store(diskgraph_store::StoreError::Io(error)))?,
                std::env::current_dir().unwrap_or_default(),
            );
            add(config, &registration, *apply_config).map(|applied| applied.to_json("add"))
        }
        InstallCommand::Show { client, config } => {
            require_client(client).map_err(|error| {
                EngineError::Store(diskgraph_store::StoreError::InvalidGraph(error.to_string()))
            })?;
            show(config)
        }
        InstallCommand::Remove {
            client,
            config,
            apply_config,
        } => {
            require_client(client).map_err(|error| {
                EngineError::Store(diskgraph_store::StoreError::InvalidGraph(error.to_string()))
            })?;
            remove(config, *apply_config).map(|applied| applied.to_json("remove"))
        }
    };
    match outcome {
        Ok(payload) => {
            let envelope = Envelope::ok(payload);
            println!("{}", serde_json::to_string(&envelope).unwrap_or_default());
            Ok(())
        }
        Err(error) => Err(EngineError::Store(
            diskgraph_store::StoreError::InvalidGraph(error.to_string()),
        )),
    }
}
