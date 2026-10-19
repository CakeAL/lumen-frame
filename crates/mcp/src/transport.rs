use crate::McpServer;
use anyhow::Result;
use lumen_frame_core::{ErrorCode, OperationError, Reply, Session, bridge::command_channel};
use rmcp::{ServiceExt, transport::stdio};

/// 独立进程只转发命令，不创建第二份桌面状态；断线后下次请求重新连接，不重试已发送操作。
pub async fn serve_desktop_stdio(endpoint: crate::ipc::DesktopEndpoint) -> Result<()> {
    let (client, inbox) = command_channel();
    let server = McpServer::new(client)?;
    let forwarding = tokio::spawn(async move {
        while let Some(call) = inbox.next().await {
            if call.is_cancelled() {
                continue;
            }
            let (request, response) = call.into_parts();
            let result = endpoint.call(request);
            tokio::pin!(result);
            // MCP 取消后关闭 IPC，让桌面跳过尚未开始的操作。
            tokio::select! {
                result = &mut result => {
                    let reply = result.unwrap_or_else(|error| Reply {
                        revision: 0,
                        result: Err(OperationError {
                            code: ErrorCode::OperationFailed,
                            message: format!("桌面连接失败：{error:#}；请打开 Lumen Frame，再重新读取状态。已发送的操作不会自动重试。"),
                        }),
                    });
                    let _ = response.try_send(reply);
                }
                _ = response.closed() => {}
            }
        }
    });
    let result = server.serve(stdio()).await;
    let result = match result {
        Ok(service) => service.waiting().await.map(|_| ()).map_err(Into::into),
        Err(error) => Err(error.into()),
    };
    forwarding.abort();
    result
}

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
