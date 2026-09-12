# 架构与模块边界

## 分层

项目保持单一 Rust crate，但按依赖方向划分为以下层次：

```text
main（启动与平台适配）
        ↓
ui / theme（GPUI 展示、控件状态、后台任务编排）
        ↓
workspace（照片队列、稳定身份、选择策略）
        ↓
params / config / photo / process（领域数据、持久化、图像处理）
        ↓
libvips、文件系统、GPUI 平台
```

- `src/main.rs` 只负责应用启动、窗口和已打包应用的 vips 模块定位。
- `src/ui/` 是展示层。`AppView` 拥有 GPUI 控件实体、订阅、预览实体、缩略图位图缓存和导出进度；它编排后台任务，但不定义照片队列的选择规则。
- `src/workspace.rs` 是应用状态层。它不依赖 GPUI 或 `RenderImage`，只管理照片顺序、去重、`PhotoId`、选择和删除后的选择策略；可用普通单元测试覆盖。
- `src/params.rs`、`Position` 与文字水印数据是可序列化的领域输入；`src/config.rs` 是它们的 TOML 持久化适配器。
- `src/photo.rs` 和 `src/process/` 是 libvips 图像处理边界。所有 libvips 初始化仍经 `photo::ensure_vips()`，预览和导出共享 `Photo::compose_watermark`。
- `src/ui/preview_image.rs` 是刻意保留的展示适配器：它把缩放后的 vips 图像转换为 GPUI `RenderImage`，不应向队列或图像处理层泄漏该类型。

## 状态归属

| 状态 | 所有者 | 原因 |
| --- | --- | --- |
| 照片队列、选择、稳定身份 | `PhotoWorkspace` | 独立于界面、可确定地测试 |
| 缩略图与加载失败占位 | `AppView` | 仅用于 GPUI 展示，随照片移除清理 |
| 预览节奏、过期请求、预览位图 | `WatermarkPreview` 实体 | 需要跨帧任务生命周期 |
| 水印参数与文字控件投影 | `AppView` | 一个页面内的唯一编辑真值 |
| 主题/缩放偏好 | `AppView` + `config` | 前者应用到 GPUI，后者负责持久化 |

## 新代码规则

1. 新的队列规则先放入 `workspace` 并编写普通单元测试；不要让它依赖 GPUI 控件或位图。
2. 仅展示所需的缓存放在 `ui`，以稳定领域 `PhotoId` 作为键；异步结果到达时先确认该身份仍在工作区。
3. 新的图像效果放在 `process` 或 `photo`，让预览和导出复用同一处理入口。
4. 配置文件读写只通过 `config`；界面不得自行拼接用户配置目录。
5. 若某个能力再拥有独立的状态、工作流和界面，再考虑拆成 crate；当前规模下先保持模块边界，避免为单个屏幕过度拆分。
