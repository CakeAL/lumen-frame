# Changelog

## [1.0.0] - 2026-09-25

首个正式版本。

### 新增

- 为照片批量添加边框、圆角、阴影、背景和 EXIF 文字水印，并实时预览。
- 保存和加载 TOML 预设；可编辑照片的 EXIF 信息并用于预览与导出。
- 支持 Ultra HDR Gain Map 预览，以及黑白底图与彩色 Gain Map 生成。
- 借助用户安装的 FFmpeg，从视频片段生成 Motion Photo。
- 提供 macOS Apple Silicon、macOS Intel 和 Windows x86-64 独立安装包。

### 使用提示

- Motion Photo 功能需要另行安装 FFmpeg。
- macOS 使用 Homebrew 的完整 libvips；Windows web 包不支持 HEIC、AVIF 和相机 RAW。
- macOS 应用目前采用临时签名，首次运行可能需要在 Finder 中右键选择“打开”。
