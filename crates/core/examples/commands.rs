//! 一行 JSON 请求对应一行 JSON 回复；用于验证后端可由无窗口宿主调用。
//! 此示例是应用命令协议，不是 MCP 协议实现。
use lumen_frame_core::{Session, StoragePaths};
use std::io::{self, BufRead, Write};
fn main() -> anyhow::Result<()> {
    // 原生字体平台须在主线程初始化，不创建窗口。
    lumen_frame_core::initialize_native_text();
    let mut backend = Session::default();
    if let Some(root) = std::env::args_os().nth(1) {
        backend = backend.with_storage(StoragePaths::in_directory(root));
    }
    let mut output = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str(&line) {
            Ok(request) => backend.dispatch_json(request),
            Err(error) => lumen_frame_core::Reply {
                revision: backend.revision(),
                result: Err(lumen_frame_core::OperationError {
                    code: lumen_frame_core::ErrorCode::InvalidRequest,
                    message: error.to_string(),
                }),
            },
        };
        serde_json::to_writer(&mut output, &reply)?;
        writeln!(output)?;
        output.flush()?;
    }
    Ok(())
}
