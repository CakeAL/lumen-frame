#!/usr/bin/env bash
#
# 把 release 构建打成一个自带 libvips 的 macOS .app 包。
# dylibbundler 负责收集主程序和 vips 模块的非系统动态库，并将引用改为包内路径。
# 可选格式模块仍需单独复制：libvips 通过 VIPS_LIBDIR 在运行期加载它们。
#
# 用法：script/bundle-macos.sh
# 产物：dist/Lumen Frame.app

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_NAME="Lumen Frame"
EXECUTABLE="lumen-frame"
BUNDLE_ID="com.lumenframe.app"

DIST="$ROOT/dist"
APP="$DIST/$APP_NAME.app"
CONTENTS="$APP/Contents"
MACOS_DIR="$CONTENTS/MacOS"
FRAMEWORKS="$CONTENTS/Frameworks"
# libvips 到 `$VIPS_LIBDIR/vips-modules-<版本>` 下找模块，所以模块必须放在这个布局里。
VIPS_LIB_DIR="$CONTENTS/lib"
VIPS_MODULE_DIR="$VIPS_LIB_DIR/vips-modules-8.18"
# 只带界面支持列表里用得到的可选模块。libvips 自己并不直接链 libjxl，因而不带 vips-jxl。
VIPS_MODULES=(vips-heif)

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
die() { printf '\033[31m错误：\033[0m %s\n' "$*" >&2; exit 1; }

command -v dylibbundler >/dev/null 2>&1 \
    || die "找不到 dylibbundler，请先执行 brew install dylibbundler"

VIPS_PREFIX="$(brew --prefix vips 2>/dev/null || true)"
GLIB_PREFIX="$(brew --prefix glib 2>/dev/null || true)"
GETTEXT_PREFIX="$(brew --prefix gettext 2>/dev/null || true)"
[ -n "$VIPS_PREFIX" ] || die "找不到 Homebrew 的 vips，请先 brew install vips"
[ -n "$GLIB_PREFIX" ] || die "找不到 Homebrew 的 glib，请先 brew install glib"
[ -n "$GETTEXT_PREFIX" ] || die "找不到 Homebrew 的 gettext，请先 brew install gettext"

# 列出某个 Mach-O 的依赖（原始 install name 字符串）。
deps_of() {
    otool -L "$1" | tail -n +2 | sed 's/^[[:space:]]*//' | sed 's/ (compatibility.*//' | grep -v '^$' || true
}

# ---------------------------------------------------------------- 1. 构建

say "构建 release"
cargo build --release --manifest-path "$ROOT/Cargo.toml"

# ---------------------------------------------------------------- 2. 包骨架和 vips 模块

say "建立 $APP_NAME.app 骨架"
rm -rf "$APP"
mkdir -p "$MACOS_DIR" "$VIPS_MODULE_DIR" "$CONTENTS/Resources"
cp "$ROOT/target/release/$EXECUTABLE" "$MACOS_DIR/$EXECUTABLE"

cat > "$CONTENTS/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$APP_NAME</string>
    <key>CFBundleDisplayName</key><string>$APP_NAME</string>
    <key>CFBundleExecutable</key><string>$EXECUTABLE</string>
    <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>0.1.0</string>
    <key>CFBundleVersion</key><string>1</string>
    <key>LSMinimumSystemVersion</key><string>13.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSPrincipalClass</key><string>NSApplication</string>
</dict>
</plist>
PLIST

MODULE_TARGETS=()
BUNDLER_TARGET_ARGS=(-x "$MACOS_DIR/$EXECUTABLE")
for module in "${VIPS_MODULES[@]}"; do
    source_module="$VIPS_PREFIX/lib/vips-modules-8.18/$module.dylib"
    [ -e "$source_module" ] || die "找不到 vips 模块：$source_module"
    target_module="$VIPS_MODULE_DIR/$module.dylib"
    cp "$source_module" "$target_module"
    MODULE_TARGETS+=("$target_module")
    BUNDLER_TARGET_ARGS+=(-x "$target_module")
done

# ---------------------------------------------------------------- 3. 收集并重写动态库

say "用 dylibbundler 收集并重写动态库"
# -x 同时包含主程序和运行期加载的 vips 模块；否则模块自身的依赖仍会指向构建机。
# 所有 Mach-O 都用 @executable_path 指到 Contents/Frameworks，这对主程序及其 dlopen 的模块均有效。
dylibbundler -od -b \
    "${BUNDLER_TARGET_ARGS[@]}" \
    -d "$FRAMEWORKS" \
    -p '@executable_path/../Frameworks/' \
    -s "$VIPS_PREFIX/lib" \
    -s "$GLIB_PREFIX/lib" \
    -s "$GETTEXT_PREFIX/lib"

LIB_COUNT="$(find "$FRAMEWORKS" -type f -name '*.dylib' | wc -l | tr -d ' ')"
[ "$LIB_COUNT" -gt 0 ] || die "dylibbundler 没有输出任何动态库"

# ---------------------------------------------------------------- 4. 重新签名

say "重新签名（dylibbundler 会改写 install name）"
while IFS= read -r file; do
    codesign --force --sign - --timestamp=none "$file" >/dev/null 2>&1
done < <(find "$CONTENTS" -type f \( -name '*.dylib' -o -perm -u+x \) ! -name 'Info.plist')

# ---------------------------------------------------------------- 5. 校验

say "校验：包内不应再有构建机绝对路径"
PROBLEMS=""
while IFS= read -r file; do
    while IFS= read -r dep; do
        case "$dep" in
            /usr/lib/*|/System/*) ;;
            /*)
                PROBLEMS="$PROBLEMS$(printf '%s\n    绝对路径依赖：%s\n' "${file#$APP/}" "$dep")"
                ;;
            @executable_path/*)
                relative="${dep#@executable_path/}"
                [ -e "$MACOS_DIR/$relative" ] || PROBLEMS="$PROBLEMS$(printf '%s\n    @executable_path 解析不到：%s\n' "${file#$APP/}" "$dep")"
                ;;
            @loader_path/*)
                relative="${dep#@loader_path/}"
                [ -e "$(dirname "$file")/$relative" ] || PROBLEMS="$PROBLEMS$(printf '%s\n    @loader_path 解析不到：%s\n' "${file#$APP/}" "$dep")"
                ;;
            @rpath/*)
                PROBLEMS="$PROBLEMS$(printf '%s\n    未解析的 @rpath：%s\n' "${file#$APP/}" "$dep")"
                ;;
        esac
    done < <(deps_of "$file")
done < <(find "$CONTENTS" -type f \( -name '*.dylib' -o -perm -u+x \) ! -name 'Info.plist')

if [ -n "$PROBLEMS" ]; then
    printf '%s' "$PROBLEMS" | sed 's/^/  /'
    die "包内仍有无法在包内解析的动态库依赖，换台机器会加载失败"
fi
say "  依赖全部指向包内"

say "完成：$APP"
printf '  体积：%s\n' "$(du -sh "$APP" | cut -f1)"
printf '  库：%s 个 + %s 个 vips 模块\n' "$LIB_COUNT" "${#MODULE_TARGETS[@]}"
printf '\n用 open "%s" 试运行；第一次打开若被 Gatekeeper 拦下，右键 → 打开。\n' "$APP"
