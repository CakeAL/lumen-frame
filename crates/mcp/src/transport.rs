use crate::McpServer;
use anyhow::Result;
use lumen_frame_core::{Session, bridge::command_channel};
use rmcp::{ServiceExt, transport::stdio};

/// 无窗口宿主须先在主线程初始化原生字体；日志只能写 stderr。
pub async fn serve_stdio(mut session: Session) -> Result<()> {
    let (client, inbox) = command_channel();
    let server = McpServer::new(client)?;
    std::thread::Builder::new()
        .name("lumen-mcp-commands".into())
        .spawn(move || {
            while let Some(call) = inbox.next_blocking() {
                if call.is_cancelled() {
                    continue;
                }
                let (request, response) = call.into_parts();
                let _ = response.try_send(session.dispatch(request));
            }
        })?;
    server.serve(stdio()).await?.waiting().await?;
    Ok(())
}
