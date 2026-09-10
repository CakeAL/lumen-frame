//! 内置主题：内嵌的 JSON 能被解析、能按明暗筛出来、能在注册表里查到，并且真的会换掉
//! 当前生效的配色。
//!
//! 主题是编译期内嵌的，所以这条链路断了不会被编译器发现 —— 只会在运行时表现为「设置里
//! 的下拉是空的」。

use gpui_kit::TestAppContext;
use gpui_kit::component::{ActiveTheme as _, Theme, ThemeMode};

use lumen_frame::theme;

#[gpui_kit::test]
fn bundled_themes_are_installed(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::install(cx);
    });

    let light = cx.update(|cx| theme::themes_for(ThemeMode::Light, cx));
    let dark = cx.update(|cx| theme::themes_for(ThemeMode::Dark, cx));

    // 注册表自带的两个默认配色。
    assert!(
        light.iter().any(|name| name.as_ref() == "Default Light"),
        "浅色列表里没有默认配色：{light:?}"
    );
    assert!(
        dark.iter().any(|name| name.as_ref() == "Default Dark"),
        "深色列表里没有默认配色：{dark:?}"
    );

    // 内嵌的主题确实装进去了，而且明暗各自都有得选。
    assert!(light.len() > 1, "浅色只有默认一套：{light:?}");
    assert!(dark.len() > 1, "深色只有默认一套：{dark:?}");

    // 顺序由框架的 `sorted_themes` 决定：默认配色置顶，其余按名字（忽略大小写）排序。
    // 下拉框依赖这个顺序，所以钉住它。
    assert_eq!(
        light.first().map(|name| name.to_string()),
        Some("Default Light".to_string()),
        "默认配色应该在浅色列表最前：{light:?}"
    );
    let rest: Vec<_> = light.iter().skip(1).cloned().collect();
    let mut by_name = rest.clone();
    by_name.sort_by_key(|name| name.to_lowercase());
    assert_eq!(rest, by_name, "默认之外的主题没有按名字排序：{light:?}");
}

#[gpui_kit::test]
fn a_bundled_theme_can_be_looked_up_and_activated(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::install(cx);
    });

    let config = cx
        .update(|cx| theme::find("Catppuccin Mocha", cx))
        .expect("内嵌主题里应该有 Catppuccin Mocha");
    assert_eq!(config.mode, ThemeMode::Dark);

    // 槽位机制：`apply_config` 按配色自己的 mode 写进对应槽位，
    // `Theme::change` 再从槽位取当前明暗的那一套。
    let before = cx.update(|cx| {
        Theme::global_mut(cx).apply_config(&config);
        Theme::change(ThemeMode::Dark, None, cx);
        cx.theme().theme_name().clone()
    });
    assert_eq!(
        before.as_ref(),
        "Catppuccin Mocha",
        "换了深色槽位之后当前配色没变"
    );

    // 切回浅色时走的仍然是浅色槽位，不该被深色的选择污染。
    cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
    assert_eq!(
        cx.update(|cx| cx.theme().theme_name().clone()).as_ref(),
        "Default Light",
        "浅色槽位被深色的选择带跑了"
    );
}

#[gpui_kit::test]
fn every_bundled_theme_parses(cx: &mut TestAppContext) {
    // 内嵌文件的语法错误只会在运行时被 `install` 跳过，这里把所有名字都查一遍，
    // 确认没有哪一份被静默丢掉。
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::install(cx);
    });

    let (light, dark) = cx.update(|cx| {
        (
            theme::themes_for(ThemeMode::Light, cx),
            theme::themes_for(ThemeMode::Dark, cx),
        )
    });

    for name in light.iter().chain(dark.iter()) {
        assert!(
            cx.update(|cx| theme::find(name, cx)).is_some(),
            "列表里的 {name} 在注册表里查不到"
        );
    }
}
