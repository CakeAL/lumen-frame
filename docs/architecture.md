# 架构与模块边界

项目暂时保持单一 crate，但按能力和依赖方向组织。目录不是为了给文件分类，而是为了说明
谁拥有状态、谁允许做 I/O，以及高层用例依赖哪些稳定边界。

## 依赖方向

```text
main
  ↓
ui ───────────────→ features
  ↓                    ↓
workspace / photo ─→ watermark
  ↓                    ↓
persistence        render / media / gainmap
```

依赖只能大体向下：

- `main.rs`：进程启动、窗口创建和打包后的平台定位。
- `ui/`：GPUI 状态、交互、预览任务编排与页面组合；不实现文件格式和图像算法。
- `features/`：拥有独立工作流的功能。目前包括 Motion Photo 和彩色 Gain Map。
- `workspace.rs`：照片队列、稳定 `PhotoId`、选择与删除规则，不依赖 GPUI。
- `photo.rs`：水印合成与照片导出用例；不再负责 EXIF 解析或 libvips 生命周期。
- `watermark.rs`：唯一的水印输入模型，包含参数、文字组与放置枚举；不依赖 UI、文件系统
  读写或图像后端。
- `render/`：水印的 libvips/文字渲染实现，包括画布、图片几何和 EXIF 文字排版。
- `media/`：外部媒体适配器。`vips.rs` 负责进程级 libvips 生命周期与基础解码，
  `metadata.rs` 负责 EXIF 读取。
- `gainmap.rs`：Ultra HDR gain map 元数据、MPF 标记和水印 gain map 重建。
- `persistence/`：本机 TOML 持久化；应用设置与水印预设分别位于独立模块。
- `update.rs`：无 UI 依赖的 GitHub Releases 更新后端；UI 生命周期位于
  `ui/behavior/update.rs`。

## 目录结构

```text
src/
├── main.rs
├── lib.rs
├── watermark.rs
├── workspace.rs
├── photo.rs
├── gainmap.rs
├── update.rs
├── features/
│   ├── colour_gainmap.rs
│   └── motion_photo/
│       ├── mod.rs
│       └── container.rs
├── media/
│   ├── metadata.rs
│   └── vips.rs
├── persistence/
│   ├── presets.rs
│   └── settings.rs
├── render/
│   ├── canvas.rs
│   ├── image.rs
│   └── text.rs
└── ui/
    ├── app.rs
    ├── behavior/
    ├── component/
    └── page/
```

## 状态归属

| 状态 | 所有者 |
| --- | --- |
| 照片队列、选择、稳定身份 | `PhotoWorkspace` |
| 当前照片的水印参数与文字组编辑真值 | `AppView` |
| 未选中照片的配置快照、全局新照片默认配置 | `AppView`，照片快照按 `PhotoId` 索引 |
| 缩略图缓存 | `AppView`，按 `PhotoId` 索引 |
| 水印预览节奏与过期任务 | `WatermarkPreview` 实体 |
| 小工具页的 Gain Map / Motion Photo 会话 | `OtherToolsState` |
| 应用更新 UI 生命周期 | `UpdateState` + `ui/behavior/update.rs` |
| 可持久化的应用偏好 | `persistence::settings::AppSettings` |

控件实体只保留输入、焦点、下拉开合等交互状态。业务值仍由上述所有者控制，避免第二份真值。

水印配置随照片独立保存：切换前将当前参数和文字组存为快照，切换后移出目标快照并恢复控件。
批量导出在启动时逐张冻结配置。预设默认只应用当前照片；选择「全部照片」后，载入预设会
覆盖队列并更新后续导入的默认配置，之后的手动调整仍只影响当前照片。输出目录和默认字体
始终使用本机设置，不随照片切换。移除或清空队列时一并清理对应快照。

预设面板的内置分组折叠状态与用户预设顺序属于应用偏好，保存到 `settings.toml`。
顺序以预设名为身份，忽略已移除的项，新预设追加到末尾；内置预设保持固定顺序。

文件夹导入由 `ui/behavior/import.rs` 在后台递归扫描，选择和拖放共用 `add_photos` 入口。
扫描按路径排序、过滤图片格式，并用规范化路径去重和阻止符号链接循环；扫描结果复用
原有队列、缩略图与 EXIF 加载流程。清空队列会使之前的扫描结果失效。

## 新代码规则

1. 可序列化的水印输入放在 `watermark.rs`，渲染方法放在 `render/`；持久化不得依赖渲染实现。
2. libvips 初始化和基础加载只经 `media::ensure_vips` / `media::load_base_image`。
3. EXIF 读取只经 `media::ExifInfo`；照片合成不自行解析文件容器。
4. 新的完整工作流放入 `features/`，并通过少量输入/输出类型暴露能力；不要塞回泛化的
   `process`、`helper` 或 `utils` 模块。
5. 设置与预设分别通过 `persistence::settings` 和 `persistence::presets`；UI 不拼配置路径。
6. `AppView` 只组合 feature 状态和窗口级资源。新增独立页面状态时优先创建专有状态类型，
   不继续增加一组无关字段。
7. 后台结果必须携带稳定身份或请求版本，旧任务不得覆盖新选择。
8. 只有当一个 feature 已有稳定公共接缝、独立生命周期和明显编译收益时，才拆成 Cargo crate。

## 测试边界

- `watermark`、`workspace`、时间/范围计算：纯单元测试。
- `media`、`render`、`gainmap`：真实夹具集成测试，并保留并发首次初始化测试。
- `persistence`：临时目录往返与损坏输入测试。
- UI 状态与交互：GPUI 测试上下文；图像视觉事实由预览集成测试覆盖。
