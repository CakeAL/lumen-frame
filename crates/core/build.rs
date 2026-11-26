use std::env;
use std::path::{Path, PathBuf};

const REQUIRED_IMPORT_LIBS: &[&str] = &["libvips.lib", "libglib-2.0.lib", "libgobject-2.0.lib"];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=VIPS_DEV_ROOT");
    println!("cargo:rerun-if-env-changed=LIBVIPS_LIB_DIR");

    // 仅在 Windows x64 MSVC 目标上生效，与 `.cargo/config.toml` 的
    // `[target.x86_64-pc-windows-msvc.vips]` 覆盖配对使用。
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc")
        || env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("x86_64")
    {
        return;
    }

    let workspace_root = workspace_root();
    let (source, lib_dir) = resolve_lib_dir(&workspace_root);

    let missing: Vec<&str> = REQUIRED_IMPORT_LIBS
        .iter()
        .copied()
        .filter(|lib| !lib_dir.join(lib).is_file())
        .collect();
    if !missing.is_empty() {
        panic!(
            "在 {source}（{}）中缺少 Windows MSVC 链接所需的导入库：{}。\n\
             请从 libvips/build-win64-mxe 下载并解压 x64 开发包（例如 vips-dev-x64-web-8.18.7.zip），\
             将环境变量 VIPS_DEV_ROOT 指向解压后的根目录（其下应包含 bin\\ 与 lib\\），\
             或将 LIBVIPS_LIB_DIR 指向其 lib\\ 目录。",
            lib_dir.display(),
            missing.join(", "),
        );
    }

    for lib in REQUIRED_IMPORT_LIBS {
        println!("cargo:rerun-if-changed={}", lib_dir.join(lib).display());
    }
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
}

fn workspace_root() -> PathBuf {
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or(manifest_dir)
}

fn resolve_path(raw: &str, workspace_root: &Path) -> PathBuf {
    let path = PathBuf::from(raw.trim());
    if path.is_relative() {
        let from_workspace = workspace_root.join(&path);
        if from_workspace.exists() {
            return from_workspace;
        }
    }
    path
}

fn non_empty_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn resolve_lib_dir(workspace_root: &Path) -> (&'static str, PathBuf) {
    if let Some(root) = non_empty_env("VIPS_DEV_ROOT") {
        return (
            "VIPS_DEV_ROOT",
            resolve_path(&root, workspace_root).join("lib"),
        );
    }
    if let Some(lib_dir) = non_empty_env("LIBVIPS_LIB_DIR") {
        return ("LIBVIPS_LIB_DIR", resolve_path(&lib_dir, workspace_root));
    }

    let default_lib_dir = workspace_root.join("vips-dev-8.18").join("lib");
    println!("cargo:rerun-if-changed={}", default_lib_dir.display());
    if default_lib_dir.is_dir() {
        return ("仓库根目录的 vips-dev-8.18\\lib", default_lib_dir);
    }

    panic!(
        "未找到 Windows x64 libvips 开发包。\n\
         请从 libvips/build-win64-mxe 下载并解压 x64 开发包（例如 vips-dev-x64-web-8.18.7.zip），然后任选一种方式：\n\
         1. 设置环境变量 VIPS_DEV_ROOT 指向解压后的根目录（包含 bin\\ 与 lib\\）；\n\
         2. 设置环境变量 LIBVIPS_LIB_DIR 指向其中的 lib\\ 目录；\n\
         3. 将开发包解压至仓库根目录的 {}。",
        workspace_root.join("vips-dev-8.18").display(),
    );
}
