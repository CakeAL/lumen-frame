#!/usr/bin/env bash
#
# 把 release 构建打成自带 libvips 的 macOS .app，并用 create-dmg 生成安装镜像。
#
# 用法：
#   script/bundle-macos.sh arm64      # Apple Silicon
#   script/bundle-macos.sh x86_64     # Intel
#   script/bundle-macos.sh all        # 依次构建两种架构
#
# 不传参数时构建当前 Mac 的架构。产物：
#   dist/Lumen-Frame-<版本>-macOS-arm64.dmg
#   dist/Lumen-Frame-<版本>-macOS-x86_64.dmg

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="Lumen Frame"
EXECUTABLE="lumen-frame"
BUNDLE_ID="com.lumenframe.app"
APP_VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$ROOT/Cargo.toml" | head -n 1)"
APP_BUILD_NUMBER="${APP_BUILD_NUMBER:-1}"
DIST="$ROOT/dist"
APP_ICON="$ROOT/assets/app-icon/LumenFrame.icns"
DMG_BACKGROUND="$ROOT/assets/bg.svg"
VIPS_MODULE_VERSION="8.18"
# 只带界面支持列表里用得到的可选模块。libvips 自己并不直接链 libjxl，因而不带 vips-jxl。
VIPS_MODULES=(vips-heif)

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
die() { printf '\033[31m错误：\033[0m %s\n' "$*" >&2; exit 1; }

usage() {
    cat <<'USAGE'
用法：script/bundle-macos.sh [arm64|x86_64|all]

  arm64    构建 Apple Silicon 版本
  x86_64   构建 Intel 版本
  all      依次构建两个版本（需要两套 Homebrew/libvips）

不传参数时构建当前 Mac 的架构。

在 Apple Silicon 上，脚本默认使用：
  arm64:  /opt/homebrew/bin/brew
  x86_64: /usr/local/bin/brew

可用 BREW_ARM64 或 BREW_X86_64 覆盖对应的 brew 可执行文件路径。
USAGE
}

normalize_arch() {
    case "$1" in
        arm64|aarch64) printf 'arm64\n' ;;
        x86_64|amd64) printf 'x86_64\n' ;;
        *) return 1 ;;
    esac
}

MODE="${1:-$(uname -m)}"
case "$MODE" in
    -h|--help)
        usage
        exit 0
        ;;
    all)
        ARCHES=(arm64 x86_64)
        ;;
    *)
        ARCH="$(normalize_arch "$MODE")" || {
            usage >&2
            die "未知架构：$MODE"
        }
        ARCHES=("$ARCH")
        ;;
esac

[ -n "$APP_VERSION" ] || die "无法从 Cargo.toml 读取应用版本"
[ -f "$APP_ICON" ] || die "找不到应用图标：$APP_ICON"
[ -f "$DMG_BACKGROUND" ] || die "找不到 DMG 背景：$DMG_BACKGROUND"

for required_command in cargo rustup dylibbundler create-dmg otool install_name_tool codesign lipo; do
    command -v "$required_command" >/dev/null 2>&1 \
        || die "找不到 ${required_command}，请先安装后再打包"
done

# 列出某个 Mach-O 的依赖（原始 install name 字符串）。
deps_of() {
    otool -L "$1" | tail -n +2 | sed 's/^[[:space:]]*//' | sed 's/ (compatibility.*//' | grep -v '^$' || true
}

# dylibbundler 1.0.5 可能留下重复 LC_RPATH，dyld 会拒绝加载这种库。
rpaths_of() {
    otool -l "$1" | awk '/cmd LC_RPATH/{found=1} found&&/path /{print $2; found=0}'
}

remove_rpaths() {
    local file="$1" rpath
    while IFS= read -r rpath; do
        [ -n "$rpath" ] || continue
        install_name_tool -delete_rpath "$rpath" "$file"
    done < <(rpaths_of "$file")
}

brew_for_arch() {
    local arch="$1" override candidate
    if [ "$arch" = "arm64" ]; then
        override="${BREW_ARM64:-}"
        for candidate in "$override" /opt/homebrew/bin/brew "$(command -v brew 2>/dev/null || true)"; do
            [ -n "$candidate" ] && [ -x "$candidate" ] && { printf '%s\n' "$candidate"; return; }
        done
    else
        override="${BREW_X86_64:-}"
        for candidate in "$override" /usr/local/bin/brew "$(command -v brew 2>/dev/null || true)"; do
            [ -n "$candidate" ] && [ -x "$candidate" ] && { printf '%s\n' "$candidate"; return; }
        done
    fi
    die "找不到 $arch 对应的 Homebrew；可通过 BREW_ARM64/BREW_X86_64 指定"
}

require_arch() {
    local file="$1" expected="$2" architectures
    architectures="$(lipo -archs "$file" 2>/dev/null || true)"
    case " $architectures " in
        *" $expected "*) ;;
        *) die "架构不匹配：$file 需要 $expected，实际为 ${architectures:-未知}" ;;
    esac
}

bundle_arch() {
    local arch="$1" target brew_bin vips_prefix glib_prefix gettext_prefix
    local arch_dist app contents macos_dir frameworks vips_lib_dir vips_module_dir binary
    local source_module target_module lib_count problems file rpaths dep relative dmg
    local -a module_targets bundler_target_args

    case "$arch" in
        arm64) target="aarch64-apple-darwin" ;;
        x86_64) target="x86_64-apple-darwin" ;;
        *) die "不支持的架构：$arch" ;;
    esac

    rustup target list --installed | grep -qx "$target" \
        || die "尚未安装 Rust 目标 ${target}，请执行：rustup target add $target"

    brew_bin="$(brew_for_arch "$arch")"
    vips_prefix="$($brew_bin --prefix vips 2>/dev/null || true)"
    glib_prefix="$($brew_bin --prefix glib 2>/dev/null || true)"
    gettext_prefix="$($brew_bin --prefix gettext 2>/dev/null || true)"
    [ -n "$vips_prefix" ] || die "$brew_bin 中找不到 vips，请先安装 vips"
    [ -n "$glib_prefix" ] || die "$brew_bin 中找不到 glib，请先安装 glib"
    [ -n "$gettext_prefix" ] || die "$brew_bin 中找不到 gettext，请先安装 gettext"
    require_arch "$vips_prefix/lib/libvips.42.dylib" "$arch"

    arch_dist="$DIST/macos-$arch"
    app="$arch_dist/$APP_NAME.app"
    contents="$app/Contents"
    macos_dir="$contents/MacOS"
    frameworks="$contents/Frameworks"
    # libvips 到 `$VIPS_LIBDIR/vips-modules-<版本>` 下找模块，所以模块必须放在这个布局里。
    vips_lib_dir="$contents/lib"
    vips_module_dir="$vips_lib_dir/vips-modules-$VIPS_MODULE_VERSION"
    binary="$ROOT/target/$target/release/$EXECUTABLE"

    # ---------------------------------------------------------------- 1. 构建

    say "构建 $APP_NAME ${APP_VERSION}（$arch / ${target}）"
    if [ "$arch" = "arm64" ]; then
        env MACOSX_DEPLOYMENT_TARGET=13.0 \
            CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS="-L $vips_prefix/lib -L $glib_prefix/lib -L $gettext_prefix/lib" \
            cargo build --release --target "$target" --manifest-path "$ROOT/Cargo.toml"
    else
        env MACOSX_DEPLOYMENT_TARGET=13.0 \
            CARGO_TARGET_X86_64_APPLE_DARWIN_RUSTFLAGS="-L $vips_prefix/lib -L $glib_prefix/lib -L $gettext_prefix/lib" \
            cargo build --release --target "$target" --manifest-path "$ROOT/Cargo.toml"
    fi
    [ -f "$binary" ] || die "构建完成但找不到主程序：$binary"
    require_arch "$binary" "$arch"

    # ---------------------------------------------------------------- 2. 包骨架和 vips 模块

    say "建立 $arch 的 $APP_NAME.app 骨架"
    rm -rf "$arch_dist"
    mkdir -p "$macos_dir" "$frameworks" "$vips_module_dir" "$contents/Resources"
    cp "$binary" "$macos_dir/$EXECUTABLE"
    cp "$APP_ICON" "$contents/Resources/LumenFrame.icns"

    cat > "$contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$APP_NAME</string>
    <key>CFBundleDisplayName</key><string>$APP_NAME</string>
    <key>CFBundleExecutable</key><string>$EXECUTABLE</string>
    <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleIconFile</key><string>LumenFrame</string>
    <key>CFBundleShortVersionString</key><string>$APP_VERSION</string>
    <key>CFBundleVersion</key><string>$APP_BUILD_NUMBER</string>
    <key>LSMinimumSystemVersion</key><string>13.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSPrincipalClass</key><string>NSApplication</string>
</dict>
</plist>
PLIST

    module_targets=()
    bundler_target_args=(-x "$macos_dir/$EXECUTABLE")
    for module in "${VIPS_MODULES[@]}"; do
        source_module="$vips_prefix/lib/vips-modules-$VIPS_MODULE_VERSION/$module.dylib"
        [ -e "$source_module" ] || die "找不到 vips 模块：$source_module"
        require_arch "$source_module" "$arch"
        target_module="$vips_module_dir/$module.dylib"
        cp "$source_module" "$target_module"
        module_targets+=("$target_module")
        bundler_target_args+=(-x "$target_module")
    done

    # ---------------------------------------------------------------- 3. 收集并重写动态库

    say "用 dylibbundler 收集并重写 $arch 动态库"
    # -x 同时包含主程序和运行期加载的 vips 模块；否则模块自身的依赖仍会指向构建机。
    dylibbundler -od -b \
        "${bundler_target_args[@]}" \
        -d "$frameworks" \
        -p '@executable_path/../Frameworks/' \
        -s "$vips_prefix/lib" \
        -s "$glib_prefix/lib" \
        -s "$gettext_prefix/lib"

    lib_count="$(find "$frameworks" -type f -name '*.dylib' | wc -l | tr -d ' ')"
    [ "$lib_count" -gt 0 ] || die "dylibbundler 没有输出任何动态库"

    say "移除 dylibbundler 留下的 LC_RPATH"
    while IFS= read -r file; do
        remove_rpaths "$file"
    done < <(find "$contents" -type f \( -name '*.dylib' -o -path "$macos_dir/$EXECUTABLE" \))

    # ---------------------------------------------------------------- 4. 架构校验与重新签名

    say "校验包内 Mach-O 均包含 $arch"
    while IFS= read -r file; do
        require_arch "$file" "$arch"
    done < <(find "$contents" -type f \( -name '*.dylib' -o -path "$macos_dir/$EXECUTABLE" \))

    say "重新签名（dylibbundler 会改写 install name）"
    while IFS= read -r file; do
        codesign --force --sign - --timestamp=none "$file" >/dev/null 2>&1
    done < <(find "$contents" -type f \( -name '*.dylib' -o -path "$macos_dir/$EXECUTABLE" \))
    codesign --force --sign - --timestamp=none "$app" >/dev/null 2>&1

    # ---------------------------------------------------------------- 5. 依赖校验

    say "校验：包内不应再有构建机绝对路径"
    problems=""
    while IFS= read -r file; do
        rpaths="$(rpaths_of "$file")"
        if [ -n "$rpaths" ]; then
            problems="$problems$(printf '%s\n    不应残留的 LC_RPATH：%s\n' "${file#$app/}" "$rpaths")"
        fi
        while IFS= read -r dep; do
            case "$dep" in
                /usr/lib/*|/System/*) ;;
                /*)
                    problems="$problems$(printf '%s\n    绝对路径依赖：%s\n' "${file#$app/}" "$dep")"
                    ;;
                @executable_path/*)
                    relative="${dep#@executable_path/}"
                    [ -e "$macos_dir/$relative" ] || problems="$problems$(printf '%s\n    @executable_path 解析不到：%s\n' "${file#$app/}" "$dep")"
                    ;;
                @loader_path/*)
                    relative="${dep#@loader_path/}"
                    [ -e "$(dirname "$file")/$relative" ] || problems="$problems$(printf '%s\n    @loader_path 解析不到：%s\n' "${file#$app/}" "$dep")"
                    ;;
                @rpath/*)
                    problems="$problems$(printf '%s\n    未解析的 @rpath：%s\n' "${file#$app/}" "$dep")"
                    ;;
            esac
        done < <(deps_of "$file")
    done < <(find "$contents" -type f \( -name '*.dylib' -o -path "$macos_dir/$EXECUTABLE" \))

    if [ -n "$problems" ]; then
        printf '%s' "$problems" | sed 's/^/  /'
        die "包内仍有无法在包内解析的动态库依赖，换台机器会加载失败"
    fi
    say "  依赖全部指向包内"

    # ---------------------------------------------------------------- 6. DMG

    dmg="$DIST/Lumen-Frame-$APP_VERSION-macOS-$arch.dmg"
    say "使用 create-dmg 和 assets/bg.svg 生成 $(basename "$dmg")"
    create-dmg \
        --volname "$APP_NAME" \
        --volicon "$APP_ICON" \
        --background "$DMG_BACKGROUND" \
        --window-pos 400 200 \
        --window-size 660 400 \
        --icon-size 100 \
        --icon "$APP_NAME.app" 160 185 \
        --hide-extension "$APP_NAME.app" \
        --app-drop-link 500 185 \
        --filesystem APFS \
        --format UDZO \
        --overwrite \
        "$dmg" "$arch_dist"

    [ -f "$dmg" ] || die "create-dmg 完成但没有生成：$dmg"
    say "完成：$dmg"
    printf '  架构：%s\n' "$arch"
    printf '  DMG 体积：%s\n' "$(du -sh "$dmg" | cut -f1)"
    printf '  App 体积：%s\n' "$(du -sh "$app" | cut -f1)"
    printf '  库：%s 个 + %s 个 vips 模块\n' "$lib_count" "${#module_targets[@]}"
}

mkdir -p "$DIST"
for arch in "${ARCHES[@]}"; do
    bundle_arch "$arch"
done

printf '\n全部完成。第一次打开若被 Gatekeeper 拦下，请右键应用 → 打开。\n'
