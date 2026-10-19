//! stdio 桥与桌面间的私有 IPC。仅传输 core 命令，窗口与 Session 由桌面持有。
use anyhow::{Context as _, Result, bail, ensure};
use lumen_frame_core::{Reply, Request, StoragePaths, bridge::CommandClient};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    thread::JoinHandle,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::oneshot,
    task::JoinSet,
};

const PROTOCOL: u32 = 1;
const MAX_FRAME: usize = 16 * 1024 * 1024;

#[derive(Clone)]
pub struct DesktopEndpoint {
    directory: PathBuf,
}
impl DesktopEndpoint {
    pub fn from_storage(storage: &StoragePaths) -> Result<Self> {
        Ok(Self::in_directory(
            storage
                .settings()?
                .parent()
                .context("设置目录无效")?
                .join("ipc"),
        ))
    }

    /// 用于宿主隔离或测试；同一目录只能有一个桌面所有者。
    pub fn in_directory(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    #[cfg(unix)]
    fn address(&self) -> PathBuf {
        self.directory.join("desktop.sock")
    }
    #[cfg(windows)]
    fn address(&self) -> String {
        // 固定算法而非 DefaultHasher，确保不同构建的桥进程仍能发现同一用户的桌面。
        let hash = self
            .directory
            .to_string_lossy()
            .bytes()
            .fold(0xcbf29ce484222325_u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
            });
        format!(r"\\.\pipe\lumen-frame-{hash:016x}")
    }

    pub async fn call(&self, request: Request) -> Result<Reply> {
        let token = tokio::fs::read_to_string(self.directory.join("token"))
            .await
            .context("未找到桌面连接信息，请先打开 Lumen Frame")?;
        #[cfg(unix)]
        let stream = tokio::net::UnixStream::connect(self.address())
            .await
            .context("无法连接已打开的 Lumen Frame")?;
        #[cfg(windows)]
        let stream = {
            let start = tokio::time::Instant::now();
            loop {
                match tokio::net::windows::named_pipe::ClientOptions::new().open(self.address()) {
                    Ok(stream) => break stream,
                    Err(error)
                        if error.raw_os_error() == Some(231)
                            && start.elapsed() < Duration::from_secs(5) =>
                    {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    Err(error) => return Err(error).context("无法连接已打开的 Lumen Frame"),
                }
            }
        };
        let mut stream = stream;
        write_frame(
            &mut stream,
            &Envelope {
                protocol: PROTOCOL,
                token,
                request,
            },
        )
        .await?;
        // 请求发出后不能自动重试：即使连接中断，操作也可能已经执行。
        read_frame(&mut stream)
            .await
            .context("桌面连接中断，操作结果未知")
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    protocol: u32,
    token: String,
    request: Request,
}

/// 持有监听线程与目录锁；关闭主窗口时释放，不等待媒体操作结束。
/// Unix 使用权限 0700 的目录，Windows 使用用户配置目录中的令牌并拒绝远程管道客户端。
pub struct DesktopServer {
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
    endpoint: DesktopEndpoint,
    client: CommandClient,
    _lock: File,
}
impl DesktopServer {
    pub fn start(endpoint: DesktopEndpoint, client: CommandClient) -> Result<Self> {
        prepare_directory(&endpoint.directory)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(endpoint.directory.join("lock"))?;
        lock.try_lock()
            .context("另一个 Lumen Frame 已占用桌面连接，请使用已打开的窗口")?;
        let token = rand::random::<[u8; 32]>()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let (stop, stopped) = oneshot::channel();
        let (ready, started) = std::sync::mpsc::sync_channel(1);
        let owner = endpoint.clone();
        let commands = client.clone();
        let thread = std::thread::Builder::new().name("lumen-desktop-ipc".into()).spawn(move || {
            let result = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build();
            let runtime = match result {
                Ok(runtime) => runtime,
                Err(error) => { let _ = ready.send(Err(error.into())); return; }
            };
            runtime.block_on(async move {
                let listener = match bind(&owner) {
                    Ok(listener) => listener,
                    Err(error) => { let _ = ready.send(Err(error)); return; }
                };
                if let Err(error) = write_token(&owner.directory.join("token"), &token) {
                    let _ = ready.send(Err(error));
                    return;
                }
                let _ = ready.send(Ok(()));
                let mut stopped = stopped;
                let mut clients = JoinSet::new();
                let mut listener = listener;
                loop {
                    tokio::select! {
                        _ = &mut stopped => break,
                        Some(_) = clients.join_next(), if !clients.is_empty() => {},
                        accepted = accept(&mut listener), if clients.len() < 32 => {
                            match accepted {
                                Ok(stream) => {
                                    let client = commands.clone();
                                    let token = token.clone();
                                    clients.spawn(async move { let _ = handle(stream, client, token).await; });
                                }
                                Err(error) => { eprintln!("桌面 IPC 接收失败：{error:#}"); break; }
                            }
                        }
                    }
                }
                commands.close();
                clients.abort_all();
                while clients.join_next().await.is_some() {}
            });
        })?;
        match started.recv().context("桌面 IPC 启动线程已退出")? {
            Ok(()) => Ok(Self {
                stop: Some(stop),
                thread: Some(thread),
                endpoint,
                client,
                _lock: lock,
            }),
            Err(error) => {
                let _ = thread.join();
                #[cfg(unix)]
                let _ = std::fs::remove_file(endpoint.address());
                Err(error)
            }
        }
    }
}
impl Drop for DesktopServer {
    fn drop(&mut self) {
        self.client.close();
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        #[cfg(unix)]
        let _ = std::fs::remove_file(self.endpoint.address());
        let _ = std::fs::remove_file(self.endpoint.directory.join("token"));
    }
}
fn prepare_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
        let metadata = std::fs::symlink_metadata(path)?;
        ensure!(
            metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && metadata.permissions().mode() & 0o077 == 0,
            "桌面 IPC 目录必须是当前用户的私有目录：{}",
            path.display()
        );
    }
    #[cfg(windows)]
    std::fs::create_dir_all(path)?;
    Ok(())
}
fn write_token(path: &Path, token: &str) -> Result<()> {
    use std::io::Write as _;
    let _ = std::fs::remove_file(path);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(token.as_bytes())?;
    Ok(())
}

#[cfg(unix)]
fn bind(endpoint: &DesktopEndpoint) -> Result<tokio::net::UnixListener> {
    // 目录锁已由当前进程独占，因此只能清理前一次异常退出留下的 socket。
    if endpoint.address().exists() {
        std::fs::remove_file(endpoint.address())?;
    }
    Ok(tokio::net::UnixListener::bind(endpoint.address())?)
}
#[cfg(unix)]
async fn accept(listener: &mut tokio::net::UnixListener) -> Result<tokio::net::UnixStream> {
    Ok(listener.accept().await?.0)
}
#[cfg(windows)]
struct PipeListener {
    next: tokio::net::windows::named_pipe::NamedPipeServer,
    address: String,
}
#[cfg(windows)]
fn bind(endpoint: &DesktopEndpoint) -> Result<PipeListener> {
    let address = endpoint.address();
    let next = tokio::net::windows::named_pipe::ServerOptions::new()
        .first_pipe_instance(true)
        .reject_remote_clients(true)
        .create(&address)?;
    Ok(PipeListener { next, address })
}
#[cfg(windows)]
async fn accept(
    listener: &mut PipeListener,
) -> Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    listener.next.connect().await?;
    let next = tokio::net::windows::named_pipe::ServerOptions::new()
        .reject_remote_clients(true)
        .create(&listener.address)?;
    Ok(std::mem::replace(&mut listener.next, next))
}

async fn handle<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    client: CommandClient,
    token: String,
) -> Result<()> {
    let envelope: Envelope =
        tokio::time::timeout(Duration::from_secs(5), read_frame(&mut stream)).await??;
    ensure!(
        envelope.protocol == PROTOCOL,
        "桌面与 MCP 桥版本不兼容，请使用同一版本的可执行文件"
    );
    ensure!(envelope.token == token, "桌面 IPC 身份校验失败");
    let reply = tokio::select! {
        result = client.call(envelope.request) => result?,
        // 每次连接只有一个请求；客户端关闭时丢弃排队中的操作。
        _ = stream.read_u8() => return Ok(()),
    };
    write_frame(&mut stream, &reply).await
}
async fn read_frame<S: AsyncRead + Unpin, T: DeserializeOwned>(stream: &mut S) -> Result<T> {
    let len = stream.read_u32().await? as usize;
    ensure!(len > 0 && len <= MAX_FRAME, "IPC 消息过大或为空");
    let mut bytes = vec![0; len];
    stream.read_exact(&mut bytes).await?;
    Ok(serde_json::from_slice(&bytes)?)
}
async fn write_frame<S: AsyncWrite + Unpin, T: Serialize>(stream: &mut S, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > MAX_FRAME {
        bail!("IPC 消息超过 16 MiB");
    }
    stream.write_u32(bytes.len() as u32).await?;
    stream.write_all(&bytes).await?;
    stream.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_frame_core::{Command, Session, bridge::command_channel};

    #[tokio::test]
    async fn desktop_ipc_rejects_bad_tokens_and_reconnects_after_restart() {
        let directory = std::env::temp_dir().join(format!("lf-ipc-{}", std::process::id()));
        // Unix socket 路径有长度限制，macOS 的 TMPDIR 可能很长。
        #[cfg(unix)]
        let directory = PathBuf::from("/tmp").join(directory.file_name().unwrap());
        let _ = std::fs::remove_dir_all(&directory);
        let endpoint = DesktopEndpoint::in_directory(&directory);
        let start = || {
            let (client, inbox) = lumen_frame_core::bridge::command_channel();
            let server = DesktopServer::start(endpoint.clone(), client).unwrap();
            std::thread::spawn(move || {
                let mut session = Session::default();
                while let Some(call) = inbox.next_blocking() {
                    if call.is_cancelled() {
                        continue;
                    }
                    let (request, response) = call.into_parts();
                    let _ = response.try_send(session.dispatch(request));
                }
            });
            server
        };
        let server = start();
        let token = std::fs::read_to_string(directory.join("token")).unwrap();
        let (other, _) = command_channel();
        assert!(
            DesktopServer::start(endpoint.clone(), other).is_err(),
            "同一端点不能创建第二份桌面所有者"
        );
        let request = |command| Request {
            expected_revision: None,
            command,
        };
        let reply = endpoint
            .call(request(Command::SetCustomText {
                text: "共享状态".into(),
            }))
            .await
            .unwrap();
        assert!(reply.result.is_ok());
        #[cfg(unix)]
        {
            let mut stream = tokio::net::UnixStream::connect(endpoint.address())
                .await
                .unwrap();
            write_frame(
                &mut stream,
                &Envelope {
                    protocol: PROTOCOL,
                    token: "invalid".into(),
                    request: request(Command::ClearPhotos),
                },
            )
            .await
            .unwrap();
            assert!(read_frame::<_, Reply>(&mut stream).await.is_err());
        }
        let reply = endpoint.call(request(Command::GetState)).await.unwrap();
        assert_eq!(
            reply.result.unwrap().value()["current"]["custom_text"],
            "共享状态"
        );
        drop(server);
        assert!(endpoint.call(request(Command::GetState)).await.is_err());
        let server = start();
        assert_ne!(
            std::fs::read_to_string(directory.join("token")).unwrap(),
            token
        );
        let reply = endpoint.call(request(Command::GetState)).await.unwrap();
        assert_eq!(reply.result.unwrap().value()["current"]["custom_text"], "");
        drop(server);
        let _ = std::fs::remove_dir_all(directory);
    }
}
