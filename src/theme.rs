//! 内置主题。
//!
//! gpui-kit 发布出来的 crate 只带 `Default Light` / `Default Dark` 两套，其余的配色在
//! 上游仓库的 `themes/` 目录里。这里把它们**编译期内嵌**进来：打包成 `.app` 之后没有
//! 可用的主题目录，运行期按路径去找必然失效。
//!
//! 主题文件来自 gpui-kit（Apache-2.0），配色本身多源自人们熟悉的编辑器主题。

use gpui_kit::component::{ThemeConfig, ThemeMode, ThemeRegistry};
use gpui_kit::{App, SharedString};
use std::rc::Rc;

/// 编译期内嵌的主题定义。
const BUNDLED_THEMES: &[&str] = &[
    include_str!("../assets/themes/adventure.json"),
    include_str!("../assets/themes/alduin.json"),
    include_str!("../assets/themes/asciinema.json"),
    include_str!("../assets/themes/aurora.json"),
    include_str!("../assets/themes/ayu.json"),
    include_str!("../assets/themes/catppuccin.json"),
    include_str!("../assets/themes/everforest.json"),
    include_str!("../assets/themes/fahrenheit.json"),
    include_str!("../assets/themes/flexoki.json"),
    include_str!("../assets/themes/gruvbox.json"),
    include_str!("../assets/themes/harper.json"),
    include_str!("../assets/themes/hybrid.json"),
    include_str!("../assets/themes/jellybeans.json"),
    include_str!("../assets/themes/kibble.json"),
    include_str!("../assets/themes/macos-classic.json"),
    include_str!("../assets/themes/mellifluous.json"),
    include_str!("../assets/themes/molokai.json"),
    include_str!("../assets/themes/solarized.json"),
    include_str!("../assets/themes/spaceduck.json"),
    include_str!("../assets/themes/tokyonight.json"),
    include_str!("../assets/themes/twilight.json"),
];

/// 把内置主题装进主题注册表。启动时调用一次。
///
/// 单个文件解析失败只跳过它：一份配色不该拦住应用启动。
pub fn install(cx: &mut App) {
    let registry = ThemeRegistry::global_mut(cx);
    for content in BUNDLED_THEMES {
        if let Err(error) = registry.load_themes_from_str(content) {
            eprintln!("主题解析失败，已跳过：{error:#}");
        }
    }
}

/// 某个明暗下可选的配色，按名字排序。
///
/// 从注册表读而不是从常量读：这样 `Default Light` / `Default Dark` 和用户自己放进主题
/// 目录的配色都会出现在同一个列表里。
pub fn themes_for(mode: ThemeMode, cx: &App) -> Vec<SharedString> {
    ThemeRegistry::global(cx)
        .sorted_themes()
        .into_iter()
        .filter(|config| config.mode == mode)
        .map(|config| config.name.clone())
        .collect()
}

/// 按名字取一份配色。
pub fn find(name: &str, cx: &App) -> Option<Rc<ThemeConfig>> {
    ThemeRegistry::global(cx).themes().get(name).cloned()
}
