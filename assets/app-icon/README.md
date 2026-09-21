# Lumen Frame 应用图标

图标按 macOS 27 的 Icon Composer 工作流组织，设计稿画布为 1024×1024：

- `LumenFrame.icon`：Icon Composer 工程，目标平台为 macOS。
- `02-double-frame.svg`：B 方案的双层照片边框主体。
- `03-exif-accent.svg`：表示 EXIF 文字区域的青绿色强调线。
- `LumenFrame.icns`：供当前非 Xcode 的 macOS 打包脚本使用的兼容图标。
- `LumenFrame.ico`：Windows 多尺寸图标（16、24、32、48、64、96、128、256 px），
  Windows 目标构建时由 `build.rs` 嵌入 `.exe`。

SVG 不包含圆角遮罩、阴影、模糊、高光或折射。这些外观由 macOS 27 和 Icon Composer
动态处理。编辑时应维持“背景、双框、强调线”三个组，避免把系统效果烘焙进图层。

更新回退图标后，用仓库内的无依赖工具重新封装 ICNS：

```bash
rustc script/build-macos-icns.rs -o /tmp/build-macos-icns
/tmp/build-macos-icns assets/app-icon/LumenFrame.iconset assets/app-icon/LumenFrame.icns
```

Windows ICO 也源自同一份 B 方案图形，但使用已校验的 PNG 回退稿降采样，
避免 ImageMagick 直接解析 SVG 时丢失线框。
