//! stdio MCP 协议适配；独立会话不依赖桌面 UI。
mod tools;
mod transport;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use lumen_frame_core::{Command, bridge::CommandClient};
use rmcp::{ErrorData, RoleServer, ServerHandler, model::*, service::RequestContext};
use std::{path::PathBuf, sync::Arc};
pub use transport::serve_stdio;

#[derive(Clone)]
pub struct McpServer {
    client: CommandClient,
    tools: Arc<Vec<Tool>>,
}
impl McpServer {
    pub fn new(client: CommandClient) -> anyhow::Result<Self> {
        Ok(Self {
            client,
            tools: Arc::new(tools::tools()?),
        })
    }
}
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("lumen-frame", env!("CARGO_PKG_VERSION")))
            .with_instructions("Lumen Frame 照片水印工具。通过 stdio 操作独立工作区，不连接桌面会话。先 get_state 获取 PhotoId 和 revision，修改时携带 expected_revision。下标只对应当前快照。文件路径属于运行服务的本机。")
    }
    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools.iter().find(|tool| tool.name == name).cloned()
    }
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult {
            tools: (*self.tools).clone(),
            ..Default::default()
        })
    }
    async fn call_tool(
        &self,
        params: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if self.get_tool(&params.name).is_none() {
            return Err(ErrorData::invalid_params("未知的 Lumen Frame 工具", None));
        }
        let (request, include_image) =
            tools::request(&params.name, params.arguments.unwrap_or_default())
                .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
        let image = if include_image {
            match &request.command {
                Command::RenderPreview { output, .. } | Command::RenderThumbnail { output, .. } => {
                    Some(output.clone())
                }
                _ => None,
            }
        } else {
            None
        };
        let reply = tokio::select! {
            _ = context.ct.cancelled() => return Err(ErrorData::internal_error("请求已取消", None)),
            result = self.client.call(request) => result.map_err(|error| ErrorData::internal_error(error.to_string(), None))?,
        };
        let (payload, failed) = match reply.result {
            Ok(output) => (
                serde_json::json!({"revision":reply.revision,"data":output.value()}),
                false,
            ),
            Err(error) => (
                serde_json::json!({"revision":reply.revision,"error":error}),
                true,
            ),
        };
        let mut result = if failed {
            CallToolResult::error(vec![ContentBlock::text(payload.to_string())])
        } else {
            CallToolResult::success(vec![ContentBlock::text(payload.to_string())])
        };
        result.structured_content = Some(payload);
        if !failed && let Some(path) = image {
            let content = tokio::task::spawn_blocking(move || inline_image(path))
                .await
                .ok()
                .flatten();
            if let Some(content) = content {
                result.content.push(content);
            }
        }
        Ok(result.into())
    }
}
fn inline_image(path: PathBuf) -> Option<ContentBlock> {
    if std::fs::metadata(&path).ok()?.len() > 8 * 1024 * 1024 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let mime = if bytes.starts_with(b"\x89PNG") {
        "image/png"
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        "image/jpeg"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else {
        return None;
    };
    Some(ContentBlock::image(STANDARD.encode(bytes), mime))
}
