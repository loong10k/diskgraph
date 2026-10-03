//! inventory_query：既有文件操作职责的原生 Rust 实现。
use crate::OpsError;
use crate::docker::command::run;
use crate::docker::docker_inventory::DockerInventory;
use crate::docker::docker_object::DockerObject;
use crate::docker::vm_caveat::VM_CAVEAT;
use crate::specialist::CommandRunner;
use serde_json::Value;
use std::path::Path;

/// 读取 Docker 原生对象清单。
/// 参数：runner 为现有执行器；docker 为固定程序路径。
/// 返回：分类清单及虚拟机说明或观察错误。
/// Reads Docker's own inventory. Everything here is a report; nothing removes
/// or changes anything on the daemon (7.8, EC-03).
pub fn docker_inventory(
    runner: &dyn CommandRunner,
    docker: &Path,
) -> Result<DockerInventory, OpsError> {
    let mut inventory = DockerInventory::default();
    let mut notes = vec![VM_CAVEAT.to_owned()];

    // Category aggregates straight from Docker, so the totals are Docker's
    // own accounting and not ours.
    let df = run(runner, docker, &["system", "df", "--json"])?;
    if df.exit_code == 0 {
        if let Ok(report) = serde_json::from_slice::<Value>(&df.stdout) {
            for (key, kind) in [
                ("BuildCache", "build cache"),
                ("Images", "images"),
                ("Containers", "containers"),
                ("LocalVolumes", "volumes"),
            ] {
                if let Some(entry) = report.get(key) {
                    let count = entry.get("Count").and_then(Value::as_u64);
                    let size = entry.get("Size").and_then(Value::as_u64);
                    let reclaimable = entry.get("Reclaimable").and_then(Value::as_u64);
                    notes.push(format!(
                        "docker reports {kind}: {} objects, {} bytes total, {} bytes reclaimable",
                        count.map(|c| c.to_string()).unwrap_or_else(|| "?".into()),
                        size.map(|s| s.to_string()).unwrap_or_else(|| "?".into()),
                        reclaimable
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| "?".into()),
                    ));
                }
            }
        } else {
            notes.push("docker system df output was not valid JSON; totals are unavailable".into());
        }
    } else {
        notes.push(format!(
            "docker system df exited with {}; category totals are unavailable",
            df.exit_code
        ));
    }

    // Per-object lists: one JSON object per line, exactly as Docker prints it.
    inventory.images = jsonl_objects(
        run(runner, docker, &["images", "--format", "{{json .}}"])?
            .stdout
            .as_slice(),
        |entry| {
            Some(DockerObject {
                id: entry.get("ID")?.as_str()?.to_owned(),
                kind: "image",
                detail: format!(
                    "{}:{}",
                    entry
                        .get("Repository")
                        .and_then(Value::as_str)
                        .unwrap_or("?"),
                    entry.get("Tag").and_then(Value::as_str).unwrap_or("?")
                ),
                bytes: parse_size(entry.get("Size").and_then(Value::as_str).unwrap_or("0")),
            })
        },
    );
    inventory.containers = jsonl_objects(
        run(runner, docker, &["ps", "-a", "--format", "{{json .}}"])?
            .stdout
            .as_slice(),
        |entry| {
            Some(DockerObject {
                id: entry.get("ID")?.as_str()?.to_owned(),
                kind: "container",
                detail: format!(
                    "{} ({})",
                    entry.get("Names").and_then(Value::as_str).unwrap_or("?"),
                    entry.get("State").and_then(Value::as_str).unwrap_or("?")
                ),
                bytes: parse_size(entry.get("Size").and_then(Value::as_str).unwrap_or("0")),
            })
        },
    );
    inventory.volumes = jsonl_objects(
        run(runner, docker, &["volume", "ls", "--format", "{{json .}}"])?
            .stdout
            .as_slice(),
        |entry| {
            Some(DockerObject {
                id: entry.get("Name")?.as_str()?.to_owned(),
                kind: "volume",
                // Volume sizes are not part of this listing; the aggregate is
                // the honest number we have.
                detail: format!(
                    "driver {}",
                    entry.get("Driver").and_then(Value::as_str).unwrap_or("?")
                ),
                bytes: 0,
            })
        },
    );
    notes.push(
        "volume sizes are not reported by `volume ls`; use the category total for scale".into(),
    );
    inventory.notes = notes;
    Ok(inventory)
}

/// 解析 Docker JSON 行对象。
/// 参数：output 为 JSON 行输出；map 将有效 JSON 转成可审查对象。
/// 返回：map 接受的 DockerObject 列表；无效行或未识别对象按原有逻辑忽略。
/// Parses Docker's `--format {{json .}}` output: one JSON object per line.
pub(super) fn jsonl_objects(
    output: &[u8],
    map: impl Fn(&Value) -> Option<DockerObject>,
) -> Vec<DockerObject> {
    String::from_utf8_lossy(output)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|entry| map(&entry))
        .collect()
}

/// 解析 Docker 展示字节数。
/// 参数：text 为工具报告的大小字段。
/// 返回：原有单位换算结果。
/// Docker prints sizes like `1.42GB`; a plain u64 field prints as its own
/// string. Both decode to bytes.
pub(super) fn parse_size(text: &str) -> u64 {
    let text = text.trim();
    if let Ok(bytes) = text.parse::<u64>() {
        return bytes;
    }
    let (number, unit) = text.split_at(
        text.find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(text.len()),
    );
    let value: f64 = number.parse().unwrap_or(0.0);
    let factor = match unit.trim_start() {
        "B" | "" => 1.0,
        "kB" | "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        _ => 0.0,
    };
    (value * factor) as u64
}
