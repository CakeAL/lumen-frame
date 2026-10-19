//! 进程启动方式；业务操作和传输实现分别由 core / mcp 提供。
use anyhow::{Context, Result, bail, ensure};
use std::path::PathBuf;

#[derive(Default)]
pub struct Options {
    pub mcp_stdio: bool,
    pub mcp_storage: Option<PathBuf>,
    pub help: bool,
}
impl Options {
    pub fn parse(arguments: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Self> {
        let mut options = Self::default();
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            match argument.to_str() {
                Some("--mcp-stdio") => options.mcp_stdio = true,
                Some("--mcp-storage") => {
                    options.mcp_storage = Some(
                        arguments
                            .next()
                            .context("--mcp-storage 后需要提供存储目录")?
                            .into(),
                    )
                }
                Some("--help" | "-h") => options.help = true,
                _ => bail!(
                    "无法识别启动参数：{}；使用 --help 查看用法",
                    argument.to_string_lossy()
                ),
            }
        }
        ensure!(
            options.mcp_storage.is_none() || options.mcp_stdio,
            "--mcp-storage 只用于独立 stdio 会话"
        );
        Ok(options)
    }
}
pub fn print_help() {
    println!(
        "Lumen Frame\n\n  lumen-frame                         打开桌面应用\n  lumen-frame --mcp-stdio             不打开窗口，启动独立 stdio MCP 会话\n  lumen-frame --mcp-stdio --mcp-storage 目录\n                                      使用独立设置、预设和 Logo 存储目录"
    );
}
