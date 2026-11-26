//! 工具输入直接投影 Command 的 schema；模型新增字段无需复制一份 MCP DTO。
use anyhow::{Context, Result};
use lumen_frame_core::{Command, Request, command_descriptor};
use rmcp::model::{Tool, ToolAnnotations};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub fn tools() -> Result<Vec<Tool>> {
    let schema = serde_json::to_value(schemars::schema_for!(Command))?;
    let variants = schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))
        .and_then(Value::as_array)
        .context("Command schema 缺少枚举分支")?;
    let mut tools = Vec::new();
    for variant in variants {
        let properties = &variant["properties"];
        let name = properties["method"]["const"]
            .as_str()
            .context("命令缺少 method")?;
        let descriptor =
            command_descriptor(name).with_context(|| format!("命令缺少元数据描述：{name}"))?;
        let mut input = properties
            .get("params")
            .cloned()
            .unwrap_or_else(|| json!({"type":"object","properties":{}}));
        if let Some(reference) = input.get("$ref").and_then(Value::as_str) {
            input = schema
                .pointer(reference.trim_start_matches('#'))
                .cloned()
                .context("无法解析命令参数 schema")?;
        }
        input["properties"]["expected_revision"] = json!({
            "type":"integer","minimum":0,
            "description":"可选的会话版本；必须匹配 get_state 返回的 revision，避免覆盖并发修改。"
        });
        if descriptor.supports_inline_image {
            input["properties"]["include_image"] = json!({"type":"boolean","default":true,
                "description":"返回保存路径的同时附带预览图片；大于 8 MiB 时只返回路径。"});
        }
        input["additionalProperties"] = json!(false);
        let definitions = referenced_definitions(&input, &schema);
        if !definitions.is_empty() {
            input["$defs"] = serde_json::to_value(definitions)?;
        }
        let input = input
            .as_object()
            .context("工具输入不是对象 schema")?
            .clone();
        let mut tool = Tool::new(name.to_owned(), descriptor.description, Arc::new(input));
        let mut annotations = ToolAnnotations::new().read_only(descriptor.read_only);
        annotations.destructive_hint = Some(descriptor.destructive);
        tool.annotations = Some(annotations);
        tool.output_schema = Some(Arc::new(
            json!({"type":"object","properties":{
            "revision":{"type":"integer"},"data":{},"error":{}
        },"required":["revision"]})
            .as_object()
            .unwrap()
            .clone(),
        ));
        tools.push(tool);
    }
    Ok(tools)
}

pub fn request(name: &str, mut arguments: Map<String, Value>) -> Result<(Request, bool)> {
    let expected_revision = arguments
        .remove("expected_revision")
        .map(serde_json::from_value)
        .transpose()?;
    let include_image = if command_descriptor(name).is_some_and(|d| d.supports_inline_image) {
        arguments
            .remove("include_image")
            .map(serde_json::from_value)
            .transpose()?
            .unwrap_or(true)
    } else {
        false
    };
    let command = if arguments.is_empty() {
        // Unit commands omit params; commands whose only arguments are optional still accept {}.
        serde_json::from_value(json!({"method":name}))
            .or_else(|_| serde_json::from_value(json!({"method":name,"params":arguments})))?
    } else {
        serde_json::from_value(json!({"method":name,"params":arguments}))?
    };
    Ok((
        Request {
            expected_revision,
            command,
        },
        include_image,
    ))
}

fn referenced_definitions(input: &Value, schema: &Value) -> BTreeMap<String, Value> {
    fn references(value: &Value, names: &mut BTreeSet<String>) {
        match value {
            Value::Object(fields) => {
                if let Some(name) = fields
                    .get("$ref")
                    .and_then(Value::as_str)
                    .and_then(|s| s.strip_prefix("#/$defs/"))
                {
                    names.insert(name.to_owned());
                }
                for value in fields.values() {
                    references(value, names);
                }
            }
            Value::Array(values) => {
                for value in values {
                    references(value, names);
                }
            }
            _ => {}
        }
    }
    let mut pending = BTreeSet::new();
    references(input, &mut pending);
    let mut definitions = BTreeMap::new();
    while let Some(name) = pending.pop_first() {
        if definitions.contains_key(&name) {
            continue;
        }
        if let Some(value) = schema["$defs"].get(&name) {
            references(value, &mut pending);
            definitions.insert(name, value.clone());
        }
    }
    definitions
}
