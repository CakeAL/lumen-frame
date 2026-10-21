//! 用真实 MCP 客户端验证发现、参数转换、状态冲突、图像结果；使用与 stdio 相同的字节流传输。
use lumen_frame_core::{
    Session,
    bridge::{CommandClient, command_channel},
};
use lumen_frame_mcp::McpServer;
use rmcp::{
    ServiceExt,
    model::*,
    service::{Peer, RoleClient},
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

fn host() -> (CommandClient, Arc<Mutex<Session>>) {
    let (client, inbox) = command_channel();
    let session = Arc::new(Mutex::new(Session::default()));
    let owner = session.clone();
    std::thread::spawn(move || {
        while let Some(call) = inbox.next_blocking() {
            let (request, response) = call.into_parts();
            let _ = response.try_send(owner.lock().unwrap().dispatch(request));
        }
    });
    (client, session)
}
async fn call(client: &Peer<RoleClient>, name: &str, arguments: Value) -> CallToolResult {
    let request = CallToolRequestParams::new(name.to_owned())
        .with_arguments(arguments.as_object().unwrap().clone());
    client.call_tool(request).await.unwrap()
}
#[tokio::test]
async fn tools_follow_core_schemas_and_report_conflicts_over_the_wire() {
    let (bridge, owner) = host();
    let server = McpServer::new(bridge).unwrap();
    let (read, write) = tokio::io::duplex(64 * 1024);
    let serving = tokio::spawn(async move {
        server.serve(read).await.unwrap().waiting().await.unwrap();
    });
    let client = ().serve(write).await.unwrap();
    let tools = client.list_all_tools().await.unwrap();
    assert!(tools.len() > 40);
    let state_tool = tools.iter().find(|tool| tool.name == "get_state").unwrap();
    assert!(
        !state_tool.input_schema.contains_key("$defs"),
        "无参数工具不应重复整个业务 schema"
    );
    let groups = tools
        .iter()
        .find(|tool| tool.name == "set_text_groups")
        .unwrap();
    assert!(groups.input_schema["$defs"].get("TextGroup").is_some());
    let state = call(&client, "get_state", json!({})).await;
    assert_eq!(state.structured_content.as_ref().unwrap()["revision"], 0);
    let result = call(&client,"patch_params",json!({"expected_revision":0,"changes":{"shadow_color":[255,255,255],"shadow_density":3.0}})).await;
    assert_ne!(result.is_error, Some(true));
    assert_eq!(owner.lock().unwrap().params().shadow_color, [255; 3]);
    let stale = call(
        &client,
        "set_custom_text",
        json!({"expected_revision":0,"text":"旧请求"}),
    )
    .await;
    assert_eq!(stale.is_error, Some(true));
    assert_eq!(
        stale.structured_content.unwrap()["error"]["code"],
        "conflict"
    );
    assert!(owner.lock().unwrap().params().custom_text.is_empty());
    let invalid = client
        .call_tool(
            CallToolRequestParams::new("patch_params").with_arguments(
                json!({"changes":{"shadow_density":5.0}})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(invalid.is_error, Some(true));
    // 命令内部的大配置使用 Box，不应改变 MCP 接受的 JSON 结构。
    let mut watermark = owner.lock().unwrap().current().clone();
    watermark.params.shadow_size = 0.025;
    let result = call(
        &client,
        "set_watermark",
        json!({"watermark": watermark, "all_photos": false}),
    )
    .await;
    assert_ne!(result.is_error, Some(true));
    assert_eq!(owner.lock().unwrap().params().shadow_size, 0.025);
    client.cancel().await.unwrap();
    serving.await.unwrap();
}
#[tokio::test]
async fn stdio_session_receives_a_preview_image() {
    let (bridge, owner) = host();
    let server = McpServer::new(bridge).unwrap();
    let (read, write) = tokio::io::duplex(64 * 1024);
    let serving = tokio::spawn(async move {
        server.serve(read).await.unwrap().waiting().await.unwrap();
    });
    let client = ().serve(write).await.unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test_images/ultra_hdr.jpg");
    let imported = call(&client, "import_photos", json!({"paths":[fixture]})).await;
    let id = imported.structured_content.unwrap()["data"]["added"][0].clone();
    let state = call(&client, "get_state", json!({})).await;
    assert_eq!(
        state.structured_content.unwrap()["data"]["photos"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    call(&client, "set_text_groups", json!({"groups":[]})).await;
    let scratch = Scratch::new();
    let output = scratch.0.join("preview.png");
    let image = call(
        &client,
        "render_preview",
        json!({"photo_id":id,"max_edge":240,"output":output}),
    )
    .await;
    assert_ne!(image.is_error, Some(true));
    assert!(
        image
            .content
            .iter()
            .any(|part| matches!(part, ContentBlock::Image(_)))
    );
    assert!(output.is_file());
    assert_eq!(owner.lock().unwrap().workspace().len(), 1);
    client.cancel().await.unwrap();
    serving.await.unwrap();
}
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "lumen-mcp-preview-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
