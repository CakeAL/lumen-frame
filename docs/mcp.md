# MCP 接入

应用通过官方 Rust SDK `rmcp` 提供 MCP 工具。协议适配位于 `crates/mcp`，工具输入的
JSON Schema 从 `core::Command` 自动生成。MCP 服务只使用 stdio，工作线程拥有独立的
`Session`，不依赖界面，也不连接已打开的桌面工作区。

## 独立 stdio 会话

```bash
cargo run -- --mcp-stdio
```

此模式不创建窗口；工作线程持有独立的 `Session`，从同一持久化服务载入本机偏好和 Logo。
它不连接已打开的桌面工作区。客户端应直接启动构建好的可执行文件，例如：

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

希望设置、预设和 Logo 完全独立时，增加存储目录：

```bash
cargo run -- --mcp-stdio --mcp-storage /tmp/lumen-mcp-demo
```

配置中的 `args` 也可追加 `"--mcp-storage", "/指定目录"`。此选项仅适用于独立 stdio 会话。

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

## 状态与调度

- 每个 stdio 服务进程有独立的照片队列、选中项和水印配置。
- MCP 请求经 `core::bridge` 的有界队列交给会话工作线程，按顺序执行。
- `expected_revision` 检查会话版本，避免使用过期的照片或文字组下标。
- 导出使用冻结配置，预览与导出复用相同的后端合成管线。
- 设置、预设和输出文件的写入不是事务，取消不会撤回已完成的文件操作。
- 未开始且客户端已断开的命令会跳过；已启动的媒体操作可能继续完成。

## 验证

```bash
cargo check --workspace --all-targets
cargo test -p lumen-frame-mcp --test protocol
```

协议测试覆盖字节流传输上的真实握手、工具发现、参数转换、版本冲突与预览图片返回。
