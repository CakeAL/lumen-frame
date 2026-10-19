//! 宿主通过有界命令队列接收操作；桌面主线程和无窗口工作线程复用此边界。
use crate::{Reply, Request};
use anyhow::{Result, anyhow};
use async_channel::{Receiver, Sender};

#[derive(Clone)]
pub struct CommandClient(Sender<CommandCall>);
pub struct CommandInbox(Receiver<CommandCall>);
pub struct CommandCall {
    request: Request,
    response: Sender<Reply>,
}

/// 有界队列提供背压；客户端断开后未开始的请求可以跳过。
pub fn command_channel() -> (CommandClient, CommandInbox) {
    let (sender, receiver) = async_channel::bounded(32);
    (CommandClient(sender), CommandInbox(receiver))
}
impl CommandClient {
    /// 停止接收新命令，并唤醒等待入队的调用。
    pub fn close(&self) {
        self.0.close();
    }
    pub async fn call(&self, request: Request) -> Result<Reply> {
        let (sender, receiver) = async_channel::bounded(1);
        self.0
            .send(CommandCall {
                request,
                response: sender,
            })
            .await
            .map_err(|_| anyhow!("应用会话已关闭"))?;
        receiver
            .recv()
            .await
            .map_err(|_| anyhow!("应用会话已关闭，请求未完成"))
    }
}
impl CommandInbox {
    pub async fn next(&self) -> Option<CommandCall> {
        self.0.recv().await.ok()
    }
    pub fn next_blocking(&self) -> Option<CommandCall> {
        self.0.recv_blocking().ok()
    }
}
impl CommandCall {
    pub fn is_cancelled(&self) -> bool {
        self.response.is_closed()
    }
    pub fn into_parts(self) -> (Request, Sender<Reply>) {
        (self.request, self.response)
    }
}
