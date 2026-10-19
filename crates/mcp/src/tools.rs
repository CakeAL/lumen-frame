//! 工具输入直接投影 Command 的 schema；模型新增字段无需复制一份 MCP DTO。
use anyhow::{Context, Result};
use lumen_frame_core::{Command, Request};
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
        if matches!(name, "render_preview" | "render_thumbnail") {
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
        let mut tool = Tool::new(name.to_owned(), description(name), Arc::new(input));
        let read_only = matches!(
            name,
            "get_state"
                | "get_watermark"
                | "get_exif"
                | "extract_palette"
                | "list_presets"
                | "get_settings"
                | "list_logos"
                | "inspect_video"
                | "search_places"
                | "detect_ffmpeg"
                | "validate_ffmpeg"
        );
        let mut annotations = ToolAnnotations::new().read_only(read_only);
        annotations.destructive_hint = Some(matches!(
            name,
            "remove_photo"
                | "clear_photos"
                | "delete_preset"
                | "delete_logo"
                | "clear_map_cache"
                | "install_update"
        ));
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
    let include_image = if matches!(name, "render_preview" | "render_thumbnail") {
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
fn description(name: &str) -> &'static str {
    match name {
        "get_state" => {
            "读取工作区照片、选中项、当前水印和会话版本。修改前先调用，photo_id 来自本工具返回值。"
        }
        "import_photos" => {
            "导入本机照片或目录（递归），返回新增 PhotoId 与路径错误。原文件不修改。"
        }
        "select_photo" => "切换当前工作区中的照片，桌面模式同步更新窗口。",
        "remove_photo" => "从队列移除指定照片及其配置；不删除原文件。",
        "clear_photos" => "清空工作区队列与逐照片配置；不删除原文件。",
        "get_watermark" => "读取指定照片或当前照片的完整水印；自定义文本与本机环境另列。",
        "set_watermark" => "替换完整水印样式，可应用全部照片；保留各照片自定义内容及本机环境。",
        "patch_params" => {
            "递归修改当前水印参数。先 get_watermark 查看字段；支持背景、渐变、阴影、边框、色卡。数组整体替换；gradient 是 linear-gradient(135deg,#b5b9ff,#2b2c49) 字符串。"
        }
        "set_custom_text" => "设置当前照片的自定义文本，用于 {自定义文本} 标签。",
        "set_environment" => {
            "设置会话共用的输出目录、JPEG质量(1–100)、默认字体和旋转。只改运行期；持久化请 save_settings。"
        }
        "reset_watermark" => "恢复默认水印样式，保留自定义文本和本机导出环境。",
        "set_text_groups" => {
            "替换全部文字组。组与行下标只对应当前版本，读取后使用 expected_revision。"
        }
        "add_text_group" => "添加文字组，支持位置、对齐、方向、边框和相邻依附。",
        "update_text_group" => "替换指定文字组；依附关系不得循环。",
        "remove_text_group" => "删除文字组，并修正其他组及色卡的依附下标。",
        "add_text_line" => {
            "向文字组添加模板行和行样式。模板可使用 {型号}、{ISO}、{自定义文本} 等字段。"
        }
        "update_text_line" => "替换指定模板行及其完整样式。",
        "remove_text_line" => "删除指定文字行。",
        "format_text_line" => {
            "修改字体、颜色、粗斜体、下划线或清除格式。start/end 为 UTF-8 字节选区；空选区作用于整行，触及标签时扩展为整个标签。"
        }
        "edit_text_line" => "编辑模板行文本，自动移动已有富文本样式范围。",
        "apply_current_to_all" => {
            "当前样式应用到全部照片和后续导入；保留各照片自定义内容与EXIF，色卡按各照片重新取色。"
        }
        "get_exif" => "读取队列中照片的有效 EXIF（含手动覆盖）。",
        "update_exif" => "局部修改 EXIF 覆盖值；不修改原文件。日期、分数、GPS使用可读字符串。",
        "set_gps" => "使用 WGS 84 经纬度设置指定照片的 GPS 覆盖值。",
        "extract_palette" => "提取指定照片的代表色与面积占比，支持全部 PaletteTheme。",
        "render_preview" => {
            "按指定长边分辨率生成水印预览并保存到本机路径（推荐 .png），可同时返回内联图片。"
        }
        "render_thumbnail" => "生成指定照片的缩略图并保存（推荐 .png），可同时返回内联图片。",
        "export_photos" => {
            "按冻结配置导出当前或全部照片到环境中的输出目录，返回成功输入与错误。保留 Ultra HDR gain map。"
        }
        "list_presets" => "列出内置和个人预设。",
        "save_preset" => "将当前水印保存为个人 TOML 预设；同名个人预设会覆盖，内置预设不可覆盖。",
        "load_preset" => "载入预设，可应用全部照片；保留照片内容及本机导出环境。",
        "delete_preset" => "删除个人预设文件，不能删除内置预设。",
        "get_settings" => "读取持久化应用设置。",
        "save_settings" => {
            "保存完整应用设置并同步运行期导出环境；先 get_settings 获取完整结构，避免重置未提供的偏好。"
        }
        "list_logos" => "列出内置示例和本机自定义 Logo，返回 id、name、token 和 builtin。",
        "import_logos" => {
            "导入本机 Logo 文件。names 可省略，或与 paths 等长（null 使用默认编号）；自定义名称自动追加 logo 后缀，如旅行变为 {旅行logo}。内置名称不可重复。"
        }
        "delete_logo" => {
            "按 logo_id 删除素材库副本并更新预览；保留导入源文件和模板文字，内置示例不能删除。"
        }
        "export_gainmap" => "提取输入照片的 gain map 到文件；没有 gain map 时返回 exported=false。",
        "export_colour_gainmap" => "生成黑白底图与彩色恢复 gain map 照片。",
        "inspect_video" => "使用指定 FFmpeg 路径读取视频信息。",
        "export_motion_photo" => "按时间范围与封面时间从视频生成 Motion Photo。",
        "clear_map_cache" => "删除所有地图源的本机瓦片缓存。",
        "search_places" => "通过配置的在线地点服务搜索地名。",
        "detect_ffmpeg" => "检测 PATH 和常见目录中的 FFmpeg。",
        "validate_ffmpeg" => "验证指定 FFmpeg 可执行文件。",
        "render_gainmap_preview" => "生成原图或 gain map 可视化预览并保存。",
        "render_colour_gainmap_preview" => "生成黑白底图和彩色 gain map 两张预览。",
        "render_motion_photo_cover" => "生成指定视频时间范围内的 Motion Photo 封面预览。",
        "check_update" => "联网检查 GitHub Releases 并记录检查时间。",
        "install_update" => "下载并安装新版本，会替换本机应用程序文件。",
        _ => "调用 Lumen Frame 后端操作。",
    }
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
