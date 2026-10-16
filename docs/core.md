# 后端调用与 MCP 接入边界

`lumen-frame-core` 独立拥有业务会话、模型、媒体操作与持久化；`lumen-frame-ui`
通过同一个 `Session` 和服务 API 调用它。根应用包只初始化平台并组装窗口。

后端没有前端 crate 依赖。原生字体渲染仍使用 GPUI 平台字体系统，因此它不是完全不含
GPUI 依赖的纯算法库。无窗口宿主须在主线程先调用 `initialize_native_text()`；桌面应用
使用 `render::init_font_backend(platform.text_system())` 注入已有平台的字体系统。

## 会话与调用

`Session` 管理照片队列、稳定 `PhotoId`、选中照片、各照片水印、导入默认配置和版本号。
`Session::with_storage(StoragePaths::in_directory(path))` 可将设置、预设和素材隔离到指定目录。
默认使用应用原有的系统配置目录；构造会话不会主动写入设置。

桌面 UI 使用类型化方法和后台任务；外部宿主可用 `Session::dispatch(Request)` 或
`dispatch_json(Value)`。它们最终复用相同业务状态和渲染管线，没有第二套导出逻辑。

```json
{"command":{"method":"get_state"}}
```

```json
{"expected_revision":0,"command":{"method":"patch_params","params":{"changes":{"shadow_density":2.0,"shadow_color":[255,255,255],"palette_card":{"enabled":true}}}}}
```

`patch_params` 对嵌套对象递归合并，数组整体替换，拒绝未知或本机只读字段。所有字段名和
枚举取值沿用水印模型；先用 `get_watermark` 获取当前结构。预设不会序列化自定义文本、
本机素材、输出目录、默认字体或 JPEG 质量，这些在查询结果中单独返回。

```json
{"command":{"method":"set_custom_text","params":{"text":"这张照片的说明"}}}
```

回复携带当前会话版本和结构化结果，例如：

```json
{"revision":1,"result":{"Ok":true}}
```

```json
{"revision":1,"result":{"Err":{"code":"conflict","message":"状态已更新，请重新读取后再提交"}}}
```

`expected_revision` 可省略；提供时必须匹配当前会话版本。参数及依附关系校验在提交前完成，
无效配置不会部分覆盖会话。版本号用于会话状态并发保护，不是文件系统事务版本。
文件导入和批量导出可能部分成功，结果显式返回成功项和错误；输出文件和网络操作不回滚。

## 操作覆盖

| 能力 | 命令入口 |
| --- | --- |
| 队列与选择 | `get_state`、`import_photos`、`select_photo`、`remove_photo`、`clear_photos` |
| 背景、渐变、边框、阴影、色卡、布局 | `get_watermark`、`set_watermark`、`patch_params`、`reset_watermark`、`apply_current_to_all` |
| 当前照片内容与导出环境 | `set_custom_text`、`set_environment` |
| 文字组及依附 | `set_text_groups`、`add_text_group`、`update_text_group`、`remove_text_group` |
| 文字行与富文本 | `add_text_line`、`update_text_line`、`remove_text_line`、`edit_text_line`、`format_text_line` |
| EXIF / GPS | `get_exif`、`update_exif`、`set_gps` |
| 取色 | `extract_palette`，支持 Theme 与面积占比 |
| 预览和导出 | `render_preview`、`render_thumbnail`、`export_photos` |
| 设置与预设 | `get_settings`、`save_settings`、`list_presets`、`save_preset`、`load_preset`、`delete_preset` |
| Logo 素材 | `list_logos`、`import_logos` |
| HDR 工具 | `render_gainmap_preview`、`render_colour_gainmap_preview`、`export_gainmap`、`export_colour_gainmap` |
| Motion Photo | `detect_ffmpeg`、`validate_ffmpeg`、`inspect_video`、`render_motion_photo_cover`、`export_motion_photo` |
| 地图与更新 | `clear_map_cache`、`search_places`、`check_update`、`install_update`；地图瓦片通过类型化 `geolocation::MapService` 调用 |

`format_text_line` 的选区为模板的 UTF-8 字节范围，触及 `{标签}` 时扩展为完整 token；空选区
修改整行格式。字体、颜色、粗斜体、下划线及清除格式均与桌面工具栏使用相同实现。
`edit_text_line` 自动移动已有局部样式范围。删除文字组同时修正其他文字组和色卡的依附序号。

应用到全部照片只复制样式和色卡来源规则，保留每张照片的自定义内容与 EXIF。
色卡在目标照片上重新求值；JPEG 质量、输出目录、默认字体和旋转是会话共用的本机环境。
`set_environment` 修改运行期环境，`save_settings` 同时持久化本机偏好。

## 宿主与线程

命令执行是同步的，涉及媒体、文件、FFmpeg、地图和更新时会阻塞。宿主应串行管理会话命令，
将耗时用例放到工作线程；不要在 GPUI `render` 中执行。`export_jobs` 可冻结导出快照，后续
编辑和切换照片不影响已启动任务。桌面 UI 的后台任务保持 `PhotoId` 和请求版本保护。

后端预览返回 RGBA `RasterImage`；`image::layers::PreviewLayerCache` 返回带坐标和命中区域的
缓存图层，不包含 GPUI 纹理。前端只负责 RGBA → BGRA 转换、纹理复用、窗口和控件反馈。
预览和导出共用 `Photo` 排版管线，Ultra HDR 导出继续重建 gain map。

```bash
cargo run -p lumen-frame-core --example commands -- /tmp/lumen-backend-demo
```

示例从标准输入逐行接收 JSON 请求，并逐行输出 JSON 回复。它是命令接口示例，尚未实现
MCP 的工具发现、JSON Schema、传输或与正在运行桌面会话的连接。后续 MCP 适配器应复用
这些命令；需要控制当前桌面会话时，交由其所属线程提交操作并同步控件，不能另建会话冒充。
文件选择对话框、焦点、悬停、剪贴板和系统打开目录由各宿主实现。
