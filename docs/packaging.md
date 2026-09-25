# 打包与分发

应用依赖 libvips，而 libvips 不是系统库。所以「能编译」和「能在别人机器上跑」是两件事：
后者必须把整条依赖闭包一起发出去。

两个平台的做法差别很大，原因在各自的动态加载器：

| | macOS | Windows |
| --- | --- | --- |
| 依赖怎么记 | 可执行文件里写死**绝对路径**（`/opt/homebrew/opt/vips/lib/libvips.42.dylib`） | 只记**文件名**（`libvips-42.dll`） |
| 加载器去哪找 | 按记录的那个绝对路径；找不到就报 dyld 错误 | 先搜 **exe 所在目录**，再搜 PATH |
| 所以需要做什么 | `dylibbundler` 拷库并改写 install name + 重新签名 | **把 DLL 拷到 exe 旁边就行** |

---

## macOS

```bash
brew install vips glib gettext dylibbundler create-dmg
rustup target add aarch64-apple-darwin x86_64-apple-darwin

# 每个架构在对应的 Mac 上分别构建。
script/bundle-macos.sh arm64    # Apple Silicon DMG
script/bundle-macos.sh x86_64   # Intel DMG
script/bundle-macos.sh all      # 两套环境齐全时依次构建二者
```

不传架构时构建当前 Mac 对应的版本。两种架构保持为独立安装包，不会合并成 Universal Binary：

```
dist/Lumen-Frame-<版本>-macOS-arm64.dmg
dist/Lumen-Frame-<版本>-macOS-x86_64.dmg
dist/Lumen-Frame-<版本>-aarch64-apple-darwin.zip
dist/Lumen-Frame-<版本>-x86_64-apple-darwin.zip
```

脚本使用 `create-dmg` 排列应用与 Applications 快捷方式，并直接将 660×400 的
`assets/bg.svg` 设为 Finder 背景。
同架构 ZIP 包含 `Lumen Frame.app`，供应用内的 `self_update` 使用；手动安装仍使用 DMG。

每个架构必须链接同架构的 Homebrew/libvips。默认查找位置是 Apple Silicon 的
`/opt/homebrew/bin/brew` 和 Intel 的 `/usr/local/bin/brew`，也可以用 `BREW_ARM64`、
`BREW_X86_64` 指定其它位置。若在 Apple Silicon 机器交叉构建 Intel 版本，需要另外安装
Rosetta 2 和 x86_64 Homebrew，并由后者安装 `vips`、`glib`、`gettext`。脚本会用 `lipo`
检查主程序、vips 模块及所有随附动态库，架构混用会立即报错。CI 分别在两个原生 runner 构建。

脚本做八件事：

1. 按所选架构执行 `cargo build --release --target …`
2. 建 `.app` 骨架 + `Info.plist`，并复制 `assets/app-icon/LumenFrame.icns` 作为兼容图标
3. 复制运行期加载的 `vips-heif` 模块，并将主程序和模块一并传给 `dylibbundler`
4. `dylibbundler` 递归收集非系统动态库到 `Contents/Frameworks/`，并将引用改为
   `@executable_path/../Frameworks/…`；对模块使用同一个路径是安全的，因为
   `@executable_path` 始终相对于主程序的 `Contents/MacOS/`
5. 移除 `dylibbundler` 留下的 `LC_RPATH`，再重新签名所有 `.dylib`、vips 模块与主程序，
   最后封签整个 `.app`（`codesign --force -s -`）。
   `dylibbundler` 1.0.5 可能在某些库留下重复 rpath，dyld 会拒绝加载它；它调用
   `install_name_tool` 也会让原签名失效，Apple Silicon 上不重签根本加载不了
6. 校验：包内所有 Mach-O 都是目标架构；任何漏网的绝对路径依赖、或无法在包内解析的
   `@executable_path` / `@loader_path` 都**报错退出**
7. 用 `ditto` 生成保留 `.app` 结构的 ZIP 更新包
8. 使用 `assets/bg.svg` 和 `create-dmg` 生成带 Applications 拖放入口的 DMG

`dylibbundler` 通过 Homebrew 安装。脚本将 vips、glib 与 gettext
的 `lib/` 目录作为搜索路径，因为 Homebrew 库的依赖可能是 `@rpath` 形式。

### vips 的格式模块

libvips 把 HEIC/AVIF 等可选格式做成运行期模块，查找路径来自编译期 libdir，
但 Homebrew bottle 中的路径可能指向构建机。脚本将 `vips-heif` 放在
`Contents/lib/vips-modules-8.18/`，`src/main.rs` 在初始化前将 `VIPS_LIBDIR`
指向此处。新增其它可选模块时，也要复制模块并交给 `dylibbundler` 处理依赖。
macOS 包使用完整 Homebrew libvips；具体 HEIC/AVIF/RAW 解码能力仍需用真实样张验证。

### 体积

本机 arm64 实测：完整 Homebrew libvips 包的 `.app` 约 89 MB，DMG 约 36 MB
（38,152,157 字节），包含约 70 个动态库和 `vips-heif` 模块；Intel 与 CI 产物可能不同。
不能直接从完整包中删除 libheif/libraw 等依赖，否则 dyld 可能拒绝启动。

### 还想更小的话

1. **去掉 SVG 依赖（省约 18 MB，收益最大）**
   把 26 个品牌标志在构建期预渲染成 PNG（`include_bytes!` 嵌入），圆角/阴影遮罩改成
   「预生成一张小尺寸圆角 PNG，运行时 resize」。之后 `svgload_buffer` 不再被调用，
   就可以用不带 librsvg 的 libvips 构建。代价是改代码 + 遮罩质量需要比对。
2. **使用精简版 libvips**
   可以从源码关闭不需要的格式，但会增加两种 macOS 架构的构建与维护成本。
   当前发布流程优先使用 Homebrew 的现成包。
3. 正式分发还需要 **Developer ID 签名 + 公证**（notarization），否则别人第一次打开会被
   Gatekeeper 拦下。现在的 ad-hoc 签名只适合自己人之间传。

---

## Windows

已在 Windows x64/MSVC 上用 libvips 8.18.6 开发包完成 Release 构建、DLL 导入校验、
ZIP 解压与独立目录启动测试。实际格式支持仍取决于传给脚本的 libvips 包。

### 准备 libvips

从 [build-win64-mxe v8.18.6](https://github.com/libvips/build-win64-mxe/releases/tag/v8.18.6)
下载预编译包并解压。Windows x86-64 应选择 `vips-dev-x64-web-8.18.6.zip`；
`vips-dev-arm64-web-8.18.6.zip` 是 Windows ARM64 包，不能用于本项目的 x86-64 构建。

上游提供两种变体：

- `vips-dev-x64-web-x.y.z.zip` —— 体积小，格式少
- `vips-dev-x64-all-x.y.z.zip` —— 体积大，格式全

这是 [libvips 官方安装说明](https://www.libvips.org/install.html)推荐的 Windows 安装方式，
由 [build-win64-mxe](https://github.com/libvips/build-win64-mxe) 用 MinGW-w64 容器化构建。

**两个变体的格式支持差异直接影响功能**（依据 build-win64-mxe 的依赖表）：

| | web | all |
| --- | --- | --- |
| JPEG / PNG / WebP / TIFF / GIF | ✅ | ✅ |
| **SVG（librsvg）** | ✅ | ✅ |
| EXIF / lcms | ✅ | ✅ |
| **AVIF** | ✅ | ✅ |
| **HEIC / HEIF** | 未验证 | 未验证 |
| **相机 RAW（cr2/nef/arw/dng…）** | ❌ | ✅ |
| MATLAB / HDF5 / FITS / EXR / PDF / WSI | ❌ | ✅ |

`web` 包含 libheif 和 AOM，已用 8.18.6 包验证 AVIF 编码与解码；它不含 libraw。
`src/ui/component/queue.rs` 在 Windows 上接受 AVIF，但暂不接受 HEIC 和相机 RAW，
避免尚未验证的文件进入队列后才解码失败；macOS 继续接受这些格式。

### 构建

`build.rs` 会在 Windows 目标下通过 `winresource` 把
`assets/app-icon/LumenFrame.ico` 嵌入 `lumen-frame.exe`。ICO 内含 16–256 px 的 8 组尺寸，
因此资源管理器、任务栏和快捷方式可以各自选择合适分辨率。

```powershell
# 设置 VCPKG_ROOT 和 VIPS_DEV_ROOT，再用本地 overlay 安装 vips:x64-windows。
# VIPS_DEV_ROOT 指向解压后的开发包根目录（下面有 include、lib、bin）。
$env:VCPKG_ROOT = 'C:\path\to\vcpkg'
$env:VIPS_DEV_ROOT = 'C:\path\to\vips-dev-x64-web-8.18.6'
& "$env:VCPKG_ROOT\vcpkg.exe" install vips:x64-windows --overlay-ports=./vcpkg-overlay
cargo build --release
```

### 打包

```powershell
.\script\bundle-windows.ps1 -VipsDir $env:VIPS_DEV_ROOT -Variant web -CreateZip
```

产物是 `dist\lumen-frame\` 和 `dist\Lumen-Frame-<版本>-Windows-x86_64.zip`。
ZIP 用于 Release 和应用内更新，解压后 exe 与 DLL 并排放着，
Windows 加载器优先搜 exe 所在目录，所以不需要任何改写或重签名。

脚本会扫描 exe、随附 DLL 和 vips 格式插件的导入表（`dumpbin /dependents` 或 `objdump -p`），
确认每个非系统 DLL 都在输出目录里，缺一个就报错退出；找不到检查工具时也会停止。
Windows DLL 搜索会优先查找 exe 目录，因此这里不需要 macOS 那种 install-name 改写。

### 还需要注意

- **工具链要和 libvips 的构建方式匹配**。官方包是 MinGW-w64 构建的；用
  `x86_64-pc-windows-gnu` 最省事，用 `x86_64-pc-windows-msvc` 则需要能用的 `.lib`
  导入库（官方包里有）。两者都行但别混用。
- **交叉编译不现实**。libvips 的 Windows 包和 Rust 的 Windows 链接器都得在 Windows 上，
  建议用 Windows 机器或 CI 的 `windows-latest` runner 出包。
- 配置和预设走 `dirs::config_dir()`，Windows 上是 `%APPDATA%\lumen-frame\presets`，
  代码不需要改。
- 已验证打包程序可在不包含 vcpkg 或开发包路径的 PATH 下启动；发布前仍应手动检查
  设置、常见格式的照片预览和导出。

---

## GitHub Actions Release 草稿

`.github/workflows/release.yml` 在推送 `v<版本>` 标签，或对同一标签手动触发时运行
（例如 `gh workflow run release.yml --ref v1.0.0`）。
它会核对标签与 `Cargo.toml` 的版本，并通过 `mindsers/changelog-reader-action`
读取 `CHANGELOG.md` 的对应章节，
再分别用 macOS Apple Silicon、macOS Intel 与 Windows x86-64 runner 打包。
三个构建都成功后，工作流将两个 DMG、两个 macOS 更新 ZIP 和一个 Windows ZIP
上传至同一个 **GitHub Release 草稿**，草稿正文取自该版本的 Changelog。
重跑时只更新已有草稿；若该版本已经正式发布，工作流会拒绝覆盖。

工作流需要仓库允许 GitHub Actions 创建 Release（`contents: write`）；
它不会自动把草稿发布。发布前仍须检查产物，macOS 正式分发还需 Developer ID 签名与公证。

---

## 检查清单

发布前对产物做这几件事：

- [ ] `dylibbundler` / `dumpbin` 校验通过（脚本已经做了）
- [ ] 在一台**没装 Homebrew / vips** 的机器上启动一次
- [ ] 拖入一张照片，确认预览出现（这一步才真正跑通 libvips 管线）
- [ ] 导出一次，确认输出文件正常
- [ ] macOS 用真实样张验证 HEIC/AVIF/RAW；Windows 用真实照片验证 AVIF，并确认 HEIC/RAW 不会进入队列
- [ ] 打开「设置」确认配置目录可写（预设保存）
