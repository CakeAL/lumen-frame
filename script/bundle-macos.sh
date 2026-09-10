#!/usr/bin/env bash
#
# 把 release 构建打成一个自带 libvips 的 macOS .app 包。
#
# 为什么需要这一步：二进制直接链到 Homebrew 的绝对路径（`/opt/homebrew/opt/vips/lib/
# libvips.42.dylib`），而且那 66 个依赖里有相当一部分彼此也用绝对路径互相引用。换一台
# 没装 Homebrew 的机器，dyld 在第一跳就找不到库。所以这里把整条依赖闭包拷进包内，再把
# 所有引用改写成包内相对定位（`@rpath` + `@loader_path`）。
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

# 只带我们真正会用到的可选格式模块。magick / poppler / openslide 不在界面的支持列表里，
# 带上它们会额外拖进几百 MB 的依赖。
# 只带界面支持列表里用得到的模块。libvips 自己并不直接链 libjxl，所以去掉 vips-jxl
# 之后 libjxl 那 7 个库（3.3MB）整个子树都不会进包。
VIPS_MODULES=(vips-heif)

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
die() { printf '\033[31m错误：\033[0m %s\n' "$*" >&2; exit 1; }

VIPS_PREFIX="$(brew --prefix vips 2>/dev/null || true)"
[ -n "$VIPS_PREFIX" ] || die "找不到 Homebrew 的 vips，请先 brew install vips"

# ---------------------------------------------------------------- 工具函数

# 列出某个 Mach-O 的依赖（原始 install name 字符串）。
deps_of() {
    otool -L "$1" | tail -n +2 | sed 's/^[[:space:]]*//' | sed 's/ (compatibility.*//' | grep -v '^$' || true
}

# 列出某个 Mach-O 的 LC_RPATH。
rpaths_of() {
    otool -l "$1" | awk '/cmd LC_RPATH/{f=1} f&&/path /{print $2; f=0}'
}

# 把一条依赖引用解析成磁盘上的真实文件；系统库和被排除的返回非零。
resolve_dep() {
    local owner="$1" dep="$2" base candidate rp

    case "$dep" in
        /usr/lib/*|/System/*) return 1 ;;
        @rpath/*)
            base="${dep#@rpath/}"
            while IFS= read -r rp; do
                [ -n "$rp" ] || continue
                case "$rp" in
                    @loader_path*) candidate="$(dirname "$owner")/${rp#@loader_path}/$base" ;;
                    @executable_path*) candidate="$MACOS_DIR/${rp#@executable_path}/$base" ;;
                    *) candidate="$rp/$base" ;;
                esac
                [ -e "$candidate" ] && { realpath "$candidate"; return 0; }
            done < <(rpaths_of "$owner")
            # 有些库的 @rpath 指向 Homebrew 的 opt 目录，退回标准位置再试一次。
            [ -e "$VIPS_PREFIX/lib/$base" ] && { realpath "$VIPS_PREFIX/lib/$base"; return 0; }
            return 1
            ;;
        @loader_path/*)
            candidate="$(dirname "$owner")/${dep#@loader_path/}"
            [ -e "$candidate" ] && { realpath "$candidate"; return 0; }
            return 1
            ;;
        @executable_path/*)
            candidate="$MACOS_DIR/${dep#@executable_path/}"
            [ -e "$candidate" ] && { realpath "$candidate"; return 0; }
            return 1
            ;;
        /*)
            [ -e "$dep" ] && { realpath "$dep"; return 0; }
            return 1
            ;;
    esac
    return 1
}

# ---------------------------------------------------------------- 1. 构建

say "构建 release"
cargo build --release --manifest-path "$ROOT/Cargo.toml"

# ---------------------------------------------------------------- 2. 包骨架

say "建立 $APP_NAME.app 骨架"
rm -rf "$APP"
mkdir -p "$MACOS_DIR" "$FRAMEWORKS" "$VIPS_LIB_DIR/vips-modules-8.18" "$CONTENTS/Resources"
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

# ---------------------------------------------------------------- 3. 收集依赖闭包

say "收集依赖闭包（含 vips 模块）"
declare -a SEEDS=("$MACOS_DIR/$EXECUTABLE")
for module in "${VIPS_MODULES[@]}"; do
    source_module="$VIPS_PREFIX/lib/vips-modules-8.18/$module.dylib"
    if [ -e "$source_module" ]; then
        cp "$source_module" "$VIPS_LIB_DIR/vips-modules-8.18/"
        SEEDS+=("$VIPS_LIB_DIR/vips-modules-8.18/$module.dylib")
    else
        say "  跳过不存在的模块 $module"
    fi
done

# 闭包按 realpath 去重（Homebrew 的 opt/ 是 Cellar/ 的符号链接，同一份库会出现两次）。
MANIFEST="$(mktemp)"
SEEN_FILE="$(mktemp)"
# 依赖串 → 包内文件名。必须用「依赖串」做键：同一个库在不同引用方那里可能写成
# `@rpath/libFoo-1.2.dylib` 或 `@rpath/libFoo-1.2.3.dylib` 这类不同的别名。
DEP_MAP="$(mktemp)"
trap 'rm -f "$MANIFEST" "$SEEN_FILE" "$DEP_MAP"' EXIT

QUEUE=("${SEEDS[@]}")
while [ ${#QUEUE[@]} -gt 0 ]; do
    owner="${QUEUE[0]}"
    QUEUE=("${QUEUE[@]:1}")

    while IFS= read -r dep; do
        [ -n "$dep" ] || continue
        if ! resolved="$(resolve_dep "$owner" "$dep")"; then
            continue
        fi
        printf '%s\t%s\n' "$dep" "$(basename "$resolved")" >> "$DEP_MAP"

        # macOS 自带 bash 3.2，没有关联数组，用文件当集合。
        grep -qxF "$resolved" "$SEEN_FILE" && continue
        printf '%s\n' "$resolved" >> "$SEEN_FILE"
        printf '%s\n' "$resolved" >> "$MANIFEST"
        QUEUE+=("$resolved")
    done < <(deps_of "$owner")
done

# 同名不同文件会在 Frameworks 里互相覆盖，必须显式失败而不是悄悄丢掉一个。
CLASHES="$(awk -F/ '{print $NF}' "$MANIFEST" | sort | uniq -d)"
[ -z "$CLASHES" ] || die "依赖里有同名但不同的库，需要单独处理：$CLASHES"

LIB_COUNT="$(wc -l < "$MANIFEST" | tr -d ' ')"
say "  拷入 $LIB_COUNT 个库"

while IFS= read -r lib; do
    cp "$lib" "$FRAMEWORKS/"
done < "$MANIFEST"

# ---------------------------------------------------------------- 4. 改写引用

say "改写 install name（绝对路径 → @rpath / @loader_path）"

# 把每个 Mach-O 的依赖改写成包内定位。
#   Frameworks 里的库：@rpath/<名字>，并给它自己加 @loader_path 的 rpath
#   主程序：@rpath/<名字>，rpath 指向 ../Frameworks
#   vips 模块：@rpath/<名字>，rpath 指向 ../../Frameworks
rewrite() {
    local target="$1" rpath="$2"
    local base dep mapped

    base="$(basename "$target")"
    while IFS= read -r dep; do
        [ -n "$dep" ] || continue
        case "$dep" in
            /usr/lib/*|/System/*) continue ;;
        esac

        # 查收集阶段记下的映射：不能在这里重新解析 —— 此时文件已经在包内，
        # 它的兄弟库不在原来的位置，`@rpath/...` 一定解析不到。
        mapped="$(awk -F'\t' -v d="$dep" '$1==d {print $2; exit}' "$DEP_MAP")"
        if [ -z "$mapped" ]; then
            die "依赖没能收进包里：$dep（引用方 $(basename "$target")）"
        fi

        mapped="@rpath/$mapped"
        [ "$dep" = "$mapped" ] && continue
        install_name_tool -change "$dep" "$mapped" "$target"
    done < <(deps_of "$target")

    install_name_tool -id "@rpath/$base" "$target"

    # 删掉所有绝对路径的 rpath。它们指向构建机的 Homebrew；留着的话，一旦哪个依赖漏改，
    # 在这台机器上会悄悄从 Homebrew 解析成功，等换台机器才炸 —— 那种问题最难查。
    while IFS= read -r existing; do
        case "$existing" in
            /*) install_name_tool -delete_rpath "$existing" "$target" ;;
        esac
    done < <(rpaths_of "$target")

    if [ -n "$rpath" ]; then
        while IFS= read -r existing; do
            [ "$existing" = "$rpath" ] && install_name_tool -delete_rpath "$existing" "$target"
        done < <(rpaths_of "$target")
        install_name_tool -add_rpath "$rpath" "$target"
    fi
}

while IFS= read -r lib; do
    rewrite "$FRAMEWORKS/$(basename "$lib")" "@loader_path"
done < "$MANIFEST"

for module in "$VIPS_LIB_DIR"/vips-modules-8.18/*.dylib; do
    [ -e "$module" ] || continue
    rewrite "$module" "@loader_path/../../Frameworks"
done

rewrite "$MACOS_DIR/$EXECUTABLE" "@executable_path/../Frameworks"

# ---------------------------------------------------------------- 5. 重新签名

say "重新签名（install_name_tool 会让原签名失效，Apple Silicon 上不重签就加载不了）"
while IFS= read -r lib; do
    codesign --force --sign - --timestamp=none "$FRAMEWORKS/$(basename "$lib")" >/dev/null 2>&1
done < "$MANIFEST"
for module in "$VIPS_LIB_DIR"/vips-modules-8.18/*.dylib; do
    [ -e "$module" ] || continue
    codesign --force --sign - --timestamp=none "$module" >/dev/null 2>&1
done
codesign --force --sign - --timestamp=none "$MACOS_DIR/$EXECUTABLE" >/dev/null 2>&1

# ---------------------------------------------------------------- 6. 校验

say "校验：包内不应再有任何构建机的绝对路径"
PROBLEMS=""
while IFS= read -r file; do
    bad_deps="$(deps_of "$file" | grep "^/" | grep -v "^/usr/lib" | grep -v "^/System" || true)"
    bad_rpaths="$(rpaths_of "$file" | grep "^/" || true)"
    # `@rpath/x` 必须真的能在包里找到，否则换台机器就是一条 dyld 报错。
    unresolved=""
    while IFS= read -r dep; do
        case "$dep" in
            @rpath/*)
                name="${dep#@rpath/}"
                [ -e "$FRAMEWORKS/$name" ] || [ -e "$VIPS_LIB_DIR/vips-modules-8.18/$name" ] \
                    || unresolved="$unresolved$(printf '\n    @rpath 解析不到：%s' "$name")"
                ;;
        esac
    done < <(deps_of "$file")
    bad_deps="$bad_deps$unresolved"
    if [ -n "$bad_deps" ] || [ -n "$bad_rpaths" ]; then
        PROBLEMS="$PROBLEMS$(printf '%s\n' "${file#$APP/}" "$bad_deps" "$bad_rpaths")"$'\n'
    fi
done < <(find "$CONTENTS" -type f \( -name '*.dylib' -o -perm -u+x \) ! -name 'Info.plist')

if [ -n "$PROBLEMS" ]; then
    printf '%s' "$PROBLEMS" | sed 's/^/  /'
    die "包内仍有指向构建机绝对路径的依赖或 rpath，换台机器会加载失败"
fi
say "  依赖与 rpath 全部指向包内" 

say "完成：$APP"
printf '  体积：%s\n' "$(du -sh "$APP" | cut -f1)"
printf '  库：%s 个 + %s 个 vips 模块\n' "$LIB_COUNT" "$(ls "$VIPS_LIB_DIR"/vips-modules-8.18/ | wc -l | tr -d ' ')"
printf '\n用 open "%s" 试运行；第一次打开若被 Gatekeeper 拦下，右键 → 打开。\n' "$APP"
