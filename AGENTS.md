# Lumen Frame 开发指南

## 项目概览

Lumen Frame 是一个 Rust 2024 桌面照片水印工具。它为照片添加边框、圆角、阴影、背景与 EXIF 文字水印，并支持批量队列、实时预览、主题/外观设置和可保存的 TOML 预设。

- UI：`gpui-kit`（基于 GPUI）；界面文案为中文。
- 图像处理：`libvips`；EXIF：`nom-exif`；文字排版：`parley`、`swash`、`peniko`。
- 配置：用户配置目录下的 `lumen-frame/settings.toml` 与 `lumen-frame/presets/*.toml`。
- 平台：当前重点为 macOS；提供 Windows 打包脚本，但尚未在 Windows 实机验证。

## 目录与职责

- `src/main.rs`：应用启动、窗口配置，以及打包后 `VIPS_LIBDIR` 的设置。
- `src/lib.rs`：公开模块边界；水印模型集中在 `src/watermark.rs`。
- `src/ui/`：GPUI 界面。`ui/mod.rs` 中的 `AppView` 是应用状态所有者；`preview.rs` 管理后台预览节奏；`preview_image.rs` 是图像管线到 GPUI 位图的边界。
- `src/workspace.rs`：无 GPUI 依赖的照片队列状态、稳定 `PhotoId` 与选择规则；缩略图等展示缓存不得放入这里。
- `src/photo.rs`：整条水印合成与导出用例；照片解码和 EXIF 读取分别由 `src/media/` 提供。
- `src/render/`：水印渲染细节：画布、阴影、圆角和 EXIF 文字排版。
- `src/gainmap.rs`：Ultra HDR gain map 元数据、MPF 标记和重建。
- `src/features/`：拥有独立工作流的功能，例如 Motion Photo 与彩色 Gain Map。
- `src/persistence/`：预设和应用设置的 TOML 持久化；`src/watermark.rs` 是水印输入模型。
- `src/theme.rs` 与 `assets/themes/`：内置主题注册及主题 JSON。
- `static/logo/`：EXIF 相机品牌对应的黑白 SVG 标志。
- `tests/`：集成测试；`test_images/` 是受版本控制的真实图片夹具。
- `script/bundle-macos.sh`、`script/bundle-windows.ps1`：带 libvips 依赖的分发打包。macOS 用 `dylibbundler` 重写路径；Windows 将 DLL 与 exe 并排并校验整个 DLL/插件导入树；完整说明见 `docs/packaging.md`。
- `docs/architecture.md`：模块分层、状态归属与新增代码的边界规则。

## 常用命令

```bash
cargo fmt --check
cargo test
cargo run
cargo build --release
script/bundle-macos.sh
```

`cargo test` 和运行应用都需要本机可用的 libvips。macOS 打包脚本还需要 Homebrew 的 `vips`、`glib`、`gettext`、`dylibbundler` 及系统的 `otool`、`codesign`。不要把 `dist/` 或 `target/` 提交进仓库。

## 关键约束

### libvips 生命周期与并发

- **任何 libvips 调用前都必须通过 `media::ensure_vips()`（或仅从已保证它的媒体 API）初始化。** 不要在 UI 或渲染模块中自行创建/销毁 `VipsApp`。
- libvips 在首次操作时有并发初始化风险；缩略图、预览和导出会在不同后台线程运行。修改相关代码后必须运行 `cargo test --test vips_init`。
- `VipsImage` 依赖进程级 `VipsApp` 的存活期，不能引入会提前 `Drop` 该应用实例的设计。

### 预览、导出与状态

- 预览必须复用 `Photo::compose_watermark`，以确保它和导出的视觉结果一致。
- `AppView` 持有照片队列、选中项、参数和文字行；控件只保存自身交互状态，避免创建第二份业务状态。
- 后台预览的结果需遵守 `WatermarkPreview` 的去抖与过期任务处理，不能让旧任务覆盖新选择的照片。
- 队列项用 `PhotoId`，不要用 `Vec` 下标作为稳定身份。

### 预设、主题和素材

- `WatermarkParams::output_folder` 是本机环境，不属于预设。载入预设时必须保留当前输出目录。
- 预设读写只通过 `persistence::presets`；不要绕过它直接用用户输入拼路径。
- 主题的浅色和深色槽位独立保存；修改外观逻辑时要保留“跟随系统”的行为。
- 新增/移除图片格式时，同时更新 `src/ui/queue.rs` 的支持列表、测试和打包文档；Windows `web` 版 libvips 不支持 HEIC、AVIF 与相机 RAW。
- 新增相机品牌 Logo 时补齐浅/深色 SVG，并验证 `render/text.rs` 中的映射。

### Ultra HDR

- 保留输入图的 gain map；水印输出需要由 `gainmap.rs` 重建只覆盖照片区域的 gain map。
- 修改裁切、缩放、合成或导出逻辑时，使用 `test_images/ultra_hdr.jpg` 进行验证，避免将 HDR 图像悄然降为 SDR。

## 测试指引

- 业务/序列化改动：`cargo test --test presets --test settings`。
- 文字模板或 Logo 改动：`cargo test --test text`。
- 图像合成、预览或缩略图改动：`cargo test --test preview --test vips_init`。
- UI 状态、拖放、队列或设置页改动：`cargo test --test workspace --test themes`。
- 全面验证：先 `cargo fmt --check`，再 `cargo test`。

`tests/photo.rs` 会实际生成 `test_images/watermark`；它适合手动烟雾验证，但运行后不要提交生成物。

## 代码风格与变更边界

- 遵循现有 Rust 风格：`rustfmt`、`anyhow::Result` 搭配上下文、对不明显的生命周期/平台限制写出原因。
- 保持中文 UI 文案与注释的语气一致；用户可见的错误消息应可操作。
- 优先做小范围改动，避免无关重构和依赖升级。`Cargo.lock` 已受版本控制，改依赖时一并更新它。
- 不要覆盖、回退或混入已有的用户改动。开始前和结束前检查 `git status --short`；当前工作区已存在 `src/photo.rs` 的未提交修改，除非任务明确涉及它，否则保留原样。
- 修改打包脚本或 `main.rs` 的模块定位逻辑后，在干净环境验证产物；macOS 必须让 `dylibbundler` 同时处理主程序与每个运行期 vips 模块，移除它可能留下的重复 `LC_RPATH`，确保没有残留的 Homebrew 绝对 dylib 路径，并完成重签名。
