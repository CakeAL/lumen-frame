# MCP 接入

应用通过官方 Rust SDK `rmcp` 提供 MCP 工具。协议适配位于 `crates/mcp`，工具输入的
JSON Schema 从 `core::Command` 自动生成。对外只提供 stdio；默认连接已打开的桌面工作区，
AI 和界面共享同一个 `Session`，操作后同步控件、照片队列和预览。

## 桌面桥接（默认）

先正常打开 Lumen Frame，再让 MCP 客户端启动：

```bash
lumen-frame --mcp-stdio
```

```text
AI 客户端 ↔ stdio MCP 桥接进程 ↔ 本地 IPC ↔ GPUI 主线程 ↔ 当前 Session
```

桥接进程不创建照片队列或窗口。桌面关闭时，工具会返回可操作的连接错误；桌面重新打开后，
后续请求可以重新连接。桥接进程不会自动重试已经发送的操作，因为断线不代表操作没有执行。
客户端应直接启动构建好的可执行文件，例如：

```bash
cargo build --release
```

使用常见 `mcpServers` 配置格式的客户端可添加：

```json
{
  "mcpServers": {
    "lumen-frame": {
      "command": "/绝对路径/lumen-frame/target/release/lumen-frame",
      "args": ["--mcp-stdio"]
    }
  }
}
```

macOS 应用包中对应的可执行路径是 `Lumen Frame.app/Contents/MacOS/lumen-frame`（以实际
应用包名称为准）。使用该入口仍会执行原有 libvips 模块定位。
标准输出只用于 MCP 协议，日志写入标准错误。

## 独立无窗口会话

批量处理且不需要桌面联动时，显式添加 `--mcp-headless`。此时工作线程拥有独立 `Session`，
在主线程初始化字体，并从同一持久化服务载入本机偏好和 Logo：

```bash
lumen-frame --mcp-stdio --mcp-headless
```

希望设置、预设和 Logo 完全独立时，再增加存储目录：

```bash
cargo run -- --mcp-stdio --mcp-headless --mcp-storage /tmp/lumen-mcp-demo
```

配置中的 `args` 也可追加 `"--mcp-storage", "/指定目录"`。此选项仅适用于 `--mcp-headless`，避免把自定义目录误认为当前桌面。

## 工具调用

客户端通过标准 MCP 初始化和 `tools/list` 获取工具及参数定义。所有现有 `Command` 都会
投影成同名工具，包括队列、背景渐变、边框阴影、文字组与富文本、色卡、EXIF/GPS、预设、
Logo、预览导出、HDR、Motion Photo、地图和更新操作。完整业务说明见 [core.md](core.md)。

建议按以下顺序调用：

1. `get_state`：获取当前工作区和 `revision`；照片使用返回的稳定 `photo_id`。
2. `import_photos`：传入本机照片或文件夹的绝对路径，返回新增照片身份及路径错误。
3. `get_watermark`：查看完整配置结构与可用字段。
4. `patch_params` / 文字组工具：修改配置，带上刚读取的 `expected_revision`。
5. `render_preview`：返回预览；确认后调用 `export_photos`。

工具参数直接放在 MCP 调用的 `arguments` 中，不套旧命令示例的 `method` / `params` 外壳。
例如设置纯色背景：

```json
{
  "name": "patch_params",
  "arguments": {
    "changes": {
      "solid_background": true,
      "gradient_background": false,
      "background": [240, 242, 248]
    }
  }
}
```

设置渐变时，`changes` 可以包含：

```json
{
  "gradient_background": true,
  "gradient": "linear-gradient(135deg,#b5b9ff,#2b2c49)"
}
```

回复的 `structuredContent` 包含会话版本与业务数据：

```json
{"revision": 12, "data": true}
```

工具执行失败使用 MCP 的 `isError: true`，并返回可读文本和结构化错误：

```json
{"revision": 13, "error": {"code": "conflict", "message": "状态已更新，请重新读取后再提交"}}
```

参数结构错误或不存在的工具使用 JSON-RPC 错误。`expected_revision` 可以省略；若提供，
必须匹配当前会话。文字组/行下标来自当前快照，跨请求修改时应带版本。

`render_preview` 和 `render_thumbnail` 除输出路径外，默认附带 PNG/JPEG/WebP 图片内容。
图片超过 8 MiB 或无法内联时仅返回路径；`include_image: false` 可以关闭内联。
推荐预览保存为 `.png`，例如工具参数：

```json
{"photo_id": 1, "max_edge": 1600, "output": "/tmp/lumen-preview.png", "include_image": true}
```

其中 `photo_id` 必须替换为实际返回值。`export_photos` 导出到 `set_environment` 指定的
本机输出目录；`all_photos: false` 只导出当前照片。批量结果中的 `errors` 应逐项检查。

## Logo 素材

`list_logos` 返回素材的 `id`、`name`、`token`、`builtin` 与本机 `path`（内置示例为 `null`）。
`import_logos` 的 `paths` 接受本机图片路径；可选的 `names` 与文件一一对应，`null` 或空字符串
使用默认编号。自定义名称自动追加 `logo` 后缀，例如「旅行」生成 `{旅行logo}`，已有后缀不会重复添加。

```json
{"paths": ["/照片素材/旅行.svg", "/照片素材/签名.png"], "names": ["旅行", null]}
```

同名素材会拒绝导入，可换名后重试。`delete_logo` 接受 `logo_id`，只删除素材库副本，保留原文件
和模板文字；旧编号不复用。`{尼康logo}` 是无需导入即可使用的内置示例，不能删除。

## 状态与调度

- 桌面进程是唯一状态所有者；多个 stdio 桥可以连接同一桌面，命令经过有界队列串行调度。
- macOS/Linux 使用 Unix Socket，Windows 使用本机命名管道；没有 HTTP/TCP 监听端口。
- IPC 位于用户配置目录下的 `ipc` 子目录，以独占文件锁防止多桌面争用。Unix 目录权限为
  `0700`，令牌文件为 `0600`；每次启动轮换令牌，Windows 管道拒绝远程客户端。
- IPC 使用带长度的 JSON 帧，检查协议版本和令牌，单帧上限 16 MiB。
- GPUI 主线程执行状态命令；媒体与文件操作在冻结快照上后台执行，不阻塞窗口。
- 后台任务需要修改 Session 时，提交前再次检查版本；用户处理中途做的修改不会被旧快照覆盖。
- 调用前提交文字编辑草稿；调用后同步对应控件和预览。`expected_revision` 防止过期下标修改。
- 导出使用冻结配置，预览与导出复用相同的后端合成管线；桌面与 MCP 共用导出忙碌状态。
- 设置、预设和输出文件的写入不是事务，取消/版本冲突不会撤回已经完成的文件操作。
- 客户端断开后，未开始的命令可以跳过；已启动的媒体操作可能继续完成。
- 无窗口模式完全独立，不连接桌面；`--mcp-storage` 可隔离其持久化文件。

## 验证

```bash
cargo check --workspace --all-targets
cargo test -p lumen-frame-mcp
cargo test -p lumen-frame-ui behavior::commands::tests --lib
```

验证覆盖 MCP 握手、工具发现、图像返回，以及 IPC 鉴权、重复实例、重连、桌面控件/预览同步和快照冲突。
