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
brew install dylibbundler create-dmg
rustup target add aarch64-apple-darwin x86_64-apple-darwin

script/bundle-macos.sh arm64    # Apple Silicon DMG
script/bundle-macos.sh x86_64   # Intel DMG
script/bundle-macos.sh all      # 两套环境齐全时依次构建二者
```

不传架构时构建当前 Mac 对应的版本。两种架构保持为独立安装包，不会合并成 Universal Binary：

```
dist/Lumen-Frame-0.1.0-macOS-arm64.dmg
dist/Lumen-Frame-0.1.0-macOS-x86_64.dmg
```

脚本使用 `create-dmg` 排列应用与 Applications 快捷方式，并直接将 660×400 的
`assets/bg.svg` 设为 Finder 背景。

每个架构必须链接同架构的 Homebrew/libvips。默认查找位置是 Apple Silicon 的
`/opt/homebrew/bin/brew` 和 Intel 的 `/usr/local/bin/brew`，也可以用 `BREW_ARM64`、
`BREW_X86_64` 指定其它位置。若在 Apple Silicon 机器交叉构建 Intel 版本，需要另外安装
Rosetta 2 和 x86_64 Homebrew，并由后者安装 `vips`、`glib`、`gettext`。脚本会用 `lipo`
检查主程序、vips 模块及所有随附动态库，架构混用会立即报错，不会生成伪装成 Intel 的 DMG。

脚本做七件事：

1. 按所选架构执行 `cargo build --release --target …`
2. 建 `.app` 骨架 + `Info.plist`，并复制 `assets/app-icon/LumenFrame.icns` 作为兼容图标
3. 复制需要运行期加载的 vips 格式模块，并将主程序和这些模块一并传给 `dylibbundler`
4. `dylibbundler` 递归收集非系统动态库到 `Contents/Frameworks/`，并将引用改为
   `@executable_path/../Frameworks/…`；对模块使用同一个路径是安全的，因为
   `@executable_path` 始终相对于主程序的 `Contents/MacOS/`
5. 移除 `dylibbundler` 留下的 `LC_RPATH`，再重新签名所有 `.dylib`、vips 模块与主程序，
   最后封签整个 `.app`（`codesign --force -s -`）。
   `dylibbundler` 1.0.5 可能在某些库留下重复 rpath，dyld 会拒绝加载它；它调用
   `install_name_tool` 也会让原签名失效，Apple Silicon 上不重签根本加载不了
6. 校验：包内所有 Mach-O 都是目标架构；任何漏网的绝对路径依赖、或无法在包内解析的
   `@executable_path` / `@loader_path` 都**报错退出**
7. 使用 `assets/bg.svg` 和 `create-dmg` 生成带 Applications 拖放入口的 DMG

`dylibbundler` 通过 Homebrew 安装。脚本将 vips、glib 与 gettext 的 `lib/` 目录作为搜索路径，
因为 Homebrew 库的依赖可能是 `@rpath` 形式。需要新增其它运行期插件时，要把该插件也加入
脚本的 `VIPS_MODULES`，让 `dylibbundler` 同时处理它的依赖。

### vips 的格式模块

libvips 把 HEIC/AVIF 这类可选格式做成了运行期 `g_module_open` 的模块，查找路径来自
**编译期写死的 libdir**。Homebrew 的 bottle 里那个路径是构建机的：

```
VIPS-INFO: libdir = /Users/runner/work/sharp-libvips/sharp-libvips/target/lib
```

在谁的机器上都不存在。所以打包时模块放在 `Contents/lib/vips-modules-8.18/`，
`src/main.rs` 的 `point_vips_at_bundled_modules()` 在 `vips_init` 之前把 `VIPS_LIBDIR`
指到 `Contents/lib`。未打包时该目录不存在，函数不做任何事。

### 体积

打包后约 **71 MB**：

| 部分 | 体积 | 说明 |
| --- | --- | --- |
| 主程序 | 22.7 MB | GPUI 本身占大头；`Cargo.toml` 里 `strip = true` 已经压过 |
| Frameworks | 约 47 MB | 70 个库 |
| vips 模块 | 168 KB | `vips-heif` |

Frameworks 里各块的归属：

| 功能 | 体积 | 要不要 |
| --- | --- | --- |
| SVG（librsvg + cairo/pango/harfbuzz/freetype/fontconfig…，31 个库） | 17.9 MB | **必需**：`{Logo}` 和圆角/阴影遮罩都走 `svgload_buffer` |
| HEIC/AVIF（libheif + libx265 + libaom） | 14.7 MB | 看需求；libx265 是**编码器**，我们只读不写 |
| MATLAB（matio） | 4.5 MB | 用不到 |
| HDF5 | 4.1 MB | 用不到 |
| EXR（openexr + Imath + Iex + IlmThread） | 2.7 MB | 用不到 |
| 相机 RAW（libraw） | 2.5 MB | 支持 cr2/cr3/nef/arw/dng 需要 |
| FITS（cfitsio） | 1.2 MB | 用不到 |

**注意：这些（除了已去掉的 JXL）是 libvips 的硬依赖（`LC_LOAD_DYLIB`），
删文件会让 dyld 拒绝加载 libvips。** 要减只能换一个 feature 更少的 libvips 构建。

已经拿到的两处（零代码改动，81 → 71 MB）：

- `strip = true`：主程序 29.2 → 22.7 MB
- 不打包 `vips-jxl`：libvips 自己并不链 libjxl，去掉模块后那 7 个库（3.3 MB）整棵子树都不进包

### 还想更小的话

1. **去掉 SVG 依赖（省约 18 MB，收益最大）**
   把 26 个品牌标志在构建期预渲染成 PNG（`include_bytes!` 嵌入），圆角/阴影遮罩改成
   「预生成一张小尺寸圆角 PNG，运行时 resize」。之后 `svgload_buffer` 不再被调用，
   就可以用不带 librsvg 的 libvips 构建。代价是改代码 + 遮罩质量需要比对。
2. **换成 feature 更少的 libvips 构建（省约 12 MB）**
   自己用 meson 构建，关掉用不到的格式：
   ```
   meson setup build -Dmatio=disabled -Dhdf5=disabled -Dopenexr=disabled \
                     -Dcfitsio=disabled -Dpoppler=disabled -Dmagick=disabled \
                     -Dopenslide=disabled -Djxl=disabled -Dfftw=disabled
   ```
3. **去掉 HEIC/AVIF（省约 15 MB）**
   关掉 `-Dheif=disabled`，同时把这两个扩展名从界面的支持列表和文件过滤里去掉。
4. 正式分发还需要 **Developer ID 签名 + 公证**（notarization），否则别人第一次打开会被
   Gatekeeper 拦下。现在的 ad-hoc 签名只适合自己人之间传。

---

## Windows

> 这一节和 `script/bundle-windows.ps1` 是**按官方文档写的，但在 Windows 上实测过之前
> 不能算数** —— 我手上没有 Windows 环境。

### 准备 libvips

从 [libvips releases](https://github.com/libvips/libvips/releases) 下载预编译包并解压：

- `vips-dev-w64-web-x.y.z.zip` —— 体积小，格式少
- `vips-dev-w64-all-x.y.z.zip` —— 体积大，格式全

这是 [libvips 官方安装说明](https://www.libvips.org/install.html)推荐的 Windows 安装方式，
由 [build-win64-mxe](https://github.com/libvips/build-win64-mxe) 用 MinGW-w64 容器化构建。

**两个变体的格式支持差异直接影响功能**（依据 build-win64-mxe 的依赖表）：

| | web | all |
| --- | --- | --- |
| JPEG / PNG / WebP / TIFF / GIF | ✅ | ✅ |
| **SVG（librsvg）** | ✅ | ✅ |
| EXIF / lcms | ✅ | ✅ |
| **HEIC / AVIF** | ❌ | ✅ |
| **相机 RAW（cr2/nef/arw/dng…）** | ❌ | ✅ |
| MATLAB / HDF5 / FITS / EXR / PDF / WSI | ❌ | ✅ |

`web` 没有 libheif 和 libraw。如果选 `web`，**要同时把对应扩展名从
`src/ui/component/queue.rs` 的 `SUPPORTED_EXTENSIONS` 里去掉**，否则文件选择器会接受打不开的格式。

### 构建

`build.rs` 会在 Windows 目标下通过 `winresource` 把
`assets/app-icon/LumenFrame.ico` 嵌入 `lumen-frame.exe`。ICO 内含 16–256 px 的 8 组尺寸，
因此资源管理器、任务栏和快捷方式可以各自选择合适分辨率。

```
$env:RUSTFLAGS = "-L C:\vips\vips-dev-w64-web-8.18.6\lib"
cargo build --release
```

### 打包

```powershell
.\script\bundle-windows.ps1 -VipsDir C:\vips\vips-dev-w64-web-8.18.6 -Variant web
```

产物是 `dist\lumen-frame\`，直接整个目录发出去即可 —— 里面的 exe 和 DLL 并排放着，
Windows 加载器优先搜 exe 所在目录，所以不需要任何改写或重签名。

脚本会扫描 exe、随附 DLL 和 vips 格式插件的导入表（`dumpbin /dependents` 或 `objdump -p`），
确认每个非系统 DLL 都在输出目录里，缺一个就报错退出。Windows DLL 搜索会优先查找 exe
目录，因此这里不需要也不应采用 macOS 那种 install-name 改写。

### 还需要注意

- **工具链要和 libvips 的构建方式匹配**。官方包是 MinGW-w64 构建的；用
  `x86_64-pc-windows-gnu` 最省事，用 `x86_64-pc-windows-msvc` 则需要能用的 `.lib`
  导入库（官方包里有）。两者都行但别混用。
- **交叉编译不现实**。libvips 的 Windows 包和 Rust 的 Windows 链接器都得在 Windows 上，
  建议用 Windows 机器或 CI 的 `windows-latest` runner 出包。
- 配置和预设走 `dirs::config_dir()`，Windows 上是 `%APPDATA%\lumen-frame\presets`，
  代码不需要改。
- GPUI 支持 Windows（`gpui_windows::WindowsPlatform`），但**这个应用没在 Windows 上跑过**，
  首次移植要留出验证时间。

---

## 检查清单

发布前对产物做这几件事：

- [ ] `dylibbundler` / `dumpbin` 校验通过（脚本已经做了）
- [ ] 在一台**没装 Homebrew / vips** 的机器上启动一次
- [ ] 拖入一张照片，确认预览出现（这一步才真正跑通 libvips 管线）
- [ ] 导出一次，确认输出文件正常
- [ ] 如果宣称支持 HEIC/AVIF 或 RAW，各拿一张真实样张试
- [ ] 打开「设置」确认配置目录可写（预设保存）
