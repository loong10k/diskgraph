//! 工具参数契约：公告 schema 与请求校验共享同一份平面字段定义。

use serde_json::{Map, Value, json};

fn text(description: &str) -> Value {
    json!({"type":"string","minLength":1,"description":description})
}

fn unsigned(minimum: u64) -> Value {
    json!({"type":"integer","minimum":minimum})
}

/// 构建当前已服务工具的参数 schema，返回实际支持的字段和必填约束。
pub(crate) fn input_schema(catalog_id: &str) -> Value {
    let mut fields = Map::new();
    fields.insert(
        "scope".into(),
        text("Registered scope ID; revision ownership is authoritative"),
    );
    let mut required: Vec<&str> = Vec::new();
    if matches!(
        catalog_id,
        "C08" | "C09" | "C10" | "C11" | "C12" | "C13" | "C14" | "C15" | "C16"
    ) {
        fields.insert(
            "revision".into(),
            text("Published revision ID; defaults to latest when optional"),
        );
    }
    match catalog_id {
        "C01" | "C05" => {
            fields.insert(
                "action".into(),
                json!({"type":"string","enum":["list"],"default":"list"}),
            );
        }
        "C04" => {
            fields.insert(
                "job_id".into(),
                text("Durable job ID; omit for service status"),
            );
        }
        "C06" | "C07" => {
            fields.insert("before".into(), text("Earlier published revision ID"));
            fields.insert("after".into(), text("Later published revision ID"));
            required.extend(["before", "after"]);
        }
        "C08" | "C10" => {
            fields.insert("node_id".into(), unsigned(1));
        }
        "C09" => {
            fields.insert(
                "pattern".into(),
                json!({"type":"string","description":"Unicode lowercase substring pattern; empty matches all"}),
            );
            fields.insert(
                "cursor".into(),
                text("Opaque search cursor from this query"),
            );
            required.push("pattern");
        }
        "C11" | "C12" => {
            fields.insert("parent_id".into(), unsigned(1));
            fields.insert(
                "format".into(),
                json!({"type":"string","enum":["json","treemap"]}),
            );
            fields.insert(
                "width".into(),
                json!({"type":"integer","minimum":1,"maximum":4096}),
            );
            if catalog_id == "C11" {
                fields.insert("min_bytes".into(), unsigned(0));
                fields.insert(
                    "cursor".into(),
                    text("Opaque v2 directory cursor; obsolete cursors require a fresh query"),
                );
            }
        }
        "C13" | "C14" | "C15" => {
            fields.insert("entity".into(), text("Graph entity ID"));
            required.extend(["revision", "entity"]);
            if catalog_id == "C13" {
                fields.insert("relation".into(), json!({"type":"string","enum":["contains","declares","owned_by_project","owned_by_application","used_by_process","rebuildable_by","protected_by","same_content_as"]}));
                fields.insert(
                    "direction".into(),
                    json!({"type":"string","enum":["outgoing","incoming"],"default":"outgoing"}),
                );
            }
        }
        "C16" => {
            fields.insert("target_bytes".into(), unsigned(0));
        }
        _ => {}
    }
    if matches!(catalog_id, "C05" | "C09" | "C11" | "C12" | "C13" | "C14") {
        fields.insert("limit".into(), unsigned(1));
    }
    if matches!(catalog_id, "C05" | "C09" | "C11") {
        fields.insert("offset".into(), unsigned(0));
    }
    if matches!(catalog_id, "C13" | "C14") {
        fields.insert(
            "after_edge".into(),
            text("Last edge ID returned by this query"),
        );
    }
    json!({"type":"object","properties":fields,"required":required,"additionalProperties":false})
}

/// 按公告的平面 schema 校验参数；类型、未知字段和范围错误在分发前拒绝。
pub(crate) fn validate_arguments(catalog_id: &str, arguments: &Value) -> Result<(), String> {
    let args = arguments
        .as_object()
        .ok_or("tool arguments must be an object")?;
    let schema = input_schema(catalog_id);
    let properties = schema["properties"]
        .as_object()
        .expect("static schema object");
    for key in schema["required"]
        .as_array()
        .expect("static required array")
    {
        let key = key.as_str().expect("static field name");
        if !args.contains_key(key) {
            return Err(format!("missing required argument: {key}"));
        }
    }
    for (key, value) in args {
        let spec = properties
            .get(key)
            .ok_or_else(|| format!("unknown argument: {key}"))?;
        match spec["type"].as_str() {
            Some("string") => {
                let string = value
                    .as_str()
                    .ok_or_else(|| format!("{key} must be a string"))?;
                if spec["minLength"]
                    .as_u64()
                    .is_some_and(|n| string.chars().count() < n as usize)
                {
                    return Err(format!("{key} must not be empty"));
                }
            }
            Some("integer") => {
                let number = value
                    .as_u64()
                    .ok_or_else(|| format!("{key} must be an unsigned integer"))?;
                if spec["minimum"].as_u64().is_some_and(|n| number < n)
                    || spec["maximum"].as_u64().is_some_and(|n| number > n)
                {
                    return Err(format!("{key} is outside its supported range"));
                }
            }
            _ => return Err("unsupported schema field".into()),
        }
        if spec["enum"]
            .as_array()
            .is_some_and(|allowed| !allowed.contains(value))
        {
            return Err(format!("unsupported value for {key}"));
        }
    }
    Ok(())
}
