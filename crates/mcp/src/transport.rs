use crate::McpServer;
use anyhow::Result;
use lumen_frame_core::{ErrorCode, OperationError, Reply, Session, bridge::command_channel};
use rmcp::{ServiceExt, transport::stdio};
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, ReadBuf};

/// Windows 客户端可能默认用本机代码页写 stdin；rmcp 会忽略这些无效 JSON，
/// 让客户端一直等不到响应。遇到非 UTF-8 输入时关闭连接并给出可操作的诊断。
struct Utf8Input<R> {
    inner: R,
    incomplete: Vec<u8>,
}
impl<R> Utf8Input<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            incomplete: Vec::new(),
        }
    }
}
impl<R: AsyncRead + Unpin> AsyncRead for Utf8Input<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        let before = buf.filled().len();
        match Pin::new(&mut this.inner).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {}
            other => return other,
        }
        let bytes = &buf.filled()[before..];
        let eof = bytes.is_empty();
        this.incomplete.extend_from_slice(bytes);
        let valid = match std::str::from_utf8(&this.incomplete) {
            Ok(_) => {
                this.incomplete.clear();
                true
            }
            Err(error) if error.error_len().is_none() && !eof => {
                this.incomplete.drain(..error.valid_up_to());
                true
            }
            Err(_) => false,
        };
        if valid {
            Poll::Ready(Ok(()))
        } else {
            let message = "MCP stdin must be UTF-8. For PowerShell ProcessStartInfo, set StandardInputEncoding = [System.Text.UTF8Encoding]::new($false) before Start().";
            eprintln!("{message}");
            buf.set_filled(before);
            Poll::Ready(Err(io::Error::new(io::ErrorKind::InvalidData, message)))
        }
    }
}

fn utf8_stdio() -> (Utf8Input<tokio::io::Stdin>, tokio::io::Stdout) {
    let (input, output) = stdio();
    (Utf8Input::new(input), output)
}

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
    let result = server.serve(utf8_stdio()).await;
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
    server.serve(utf8_stdio()).await?.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    #[tokio::test]
    async fn utf8_input_preserves_split_characters_and_crlf() {
        let bytes = "{\"name\":\"右侧参数卡_文字加框 📷\"}\r\n".as_bytes();
        // 每次只读取一个字节，覆盖跨读取边界的 3/4 字节字符。
        let source = BufReader::with_capacity(1, bytes);
        let mut input = Utf8Input::new(source);
        let mut output = Vec::new();
        let mut byte = [0];
        while input.read(&mut byte).await.unwrap() != 0 {
            output.push(byte[0]);
        }
        assert_eq!(output, bytes);
    }

    #[tokio::test]
    async fn utf8_input_rejects_local_code_pages_and_truncated_characters() {
        for bytes in [b"{\"name\":\"\xd3\xd2\xb2\xe0\"}\n".as_slice(), b"\xe5\x8f"] {
            let mut input = Utf8Input::new(bytes);
            let error = input.read_to_end(&mut Vec::new()).await.unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(error.to_string().contains("StandardInputEncoding"));
        }
    }

    #[tokio::test]
    async fn invalid_encoding_closes_mcp_instead_of_leaving_a_request_pending() {
        let (client, _inbox) = command_channel();
        let server = McpServer::new(client).unwrap();
        let (read, peer) = tokio::io::duplex(4096);
        let (read, write) = tokio::io::split(read);
        let serving = tokio::spawn(async move {
            server
                .serve((Utf8Input::new(read), write))
                .await
                .unwrap()
                .waiting()
                .await
        });
        let mut peer = BufReader::new(peer);
        peer.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-03-26\",\"capabilities\":{},\"clientInfo\":{\"name\":\"test\",\"version\":\"1\"}}}\n")
            .await
            .unwrap();
        let mut line = String::new();
        tokio::time::timeout(std::time::Duration::from_secs(2), peer.read_line(&mut line))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&line).unwrap()["id"],
            1
        );
        peer.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"load_preset\",\"arguments\":{\"name\":\"\xd3\xd2\xb2\xe0\",\"all_photos\":false}}}\n")
            .await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), serving)
            .await
            .expect("错误编码不应导致 MCP 调用挂起")
            .unwrap()
            .unwrap();
        line.clear();
        assert_eq!(peer.read_line(&mut line).await.unwrap(), 0);
    }
}
