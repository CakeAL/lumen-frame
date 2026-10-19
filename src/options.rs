//! 进程启动方式；业务操作和传输实现分别由 core / mcp 提供。
use anyhow::{Context, Result, bail, ensure};
use std::path::PathBuf;

#[derive(Default)]
pub struct Options {
    pub mcp_stdio: bool,
    pub mcp_headless: bool,
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
                Some("--mcp-headless") => options.mcp_headless = true,
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
            !options.mcp_headless || options.mcp_stdio,
            "--mcp-headless 必须与 --mcp-stdio 一起使用"
        );
        ensure!(
            options.mcp_storage.is_none() || options.mcp_headless,
            "--mcp-storage 只用于 --mcp-stdio --mcp-headless 独立会话"
        );
        Ok(options)
    }
}
pub fn print_help() {
    println!(
        "Lumen Frame\n\n  lumen-frame                         打开桌面应用，接收本地 MCP 操作\n  lumen-frame --mcp-stdio             桥接已打开的桌面窗口\n  lumen-frame --mcp-stdio --mcp-headless\n                                      启动独立的无窗口 MCP 会话\n  lumen-frame --mcp-stdio --mcp-headless --mcp-storage 目录\n                                      使用独立设置、预设和 Logo 存储目录"
    );
}
