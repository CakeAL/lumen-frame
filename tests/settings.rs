//! 应用级偏好（目前是界面明暗）的保存与读取。
//!
//! 这份文件不该有能力拦住启动：读不到、读坏了都退回默认值。

use std::path::PathBuf;

use lumen_frame::config::{
    AppSettings, AppearanceMode, load_settings_at, save_settings_at, settings_path,
};

fn scratch_file(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lumen-frame-settings-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("settings.toml")
}

#[test]
fn default_appearance_follows_the_system() {
    // 新装的应用没有偏好文件，这时应该跟随系统，而不是自作主张选一个明暗。
    assert_eq!(AppSettings::default().appearance, AppearanceMode::System);
    assert_eq!(
        load_settings_at(&scratch_file("missing")),
        AppSettings::default()
    );
}

#[test]
fn appearance_round_trips() {
    let path = scratch_file("round-trip");

    for mode in [
        AppearanceMode::Dark,
        AppearanceMode::Light,
        AppearanceMode::System,
    ] {
        save_settings_at(
            &path,
            &AppSettings {
                appearance: mode,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            load_settings_at(&path).appearance,
            mode,
            "{mode:?} 没能往返"
        );
    }

    // 存的是可读的 TOML，不是二进制。
    let document = std::fs::read_to_string(&path).unwrap();
    assert!(document.contains("appearance"), "{document}");

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn theme_slots_round_trip() {
    let path = scratch_file("theme-slots");

    let settings = AppSettings {
        appearance: AppearanceMode::System,
        light_theme: Some("Catppuccin Latte".to_string()),
        dark_theme: Some("Catppuccin Mocha".to_string()),
    };
    save_settings_at(&path, &settings).unwrap();
    assert_eq!(load_settings_at(&path), settings);

    // 没选过主题时是 None，表示用默认的那套，而不是空字符串。
    save_settings_at(&path, &AppSettings::default()).unwrap();
    let loaded = load_settings_at(&path);
    assert_eq!(loaded.light_theme, None);
    assert_eq!(loaded.dark_theme, None);

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_broken_file_falls_back_instead_of_failing_to_start() {
    let path = scratch_file("broken");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();

    std::fs::write(&path, "这不是 TOML {{{").unwrap();
    assert_eq!(load_settings_at(&path), AppSettings::default());

    // 未知的取值同样退回默认，而不是让应用起不来。
    std::fs::write(&path, "appearance = \"Rainbow\"\n").unwrap();
    assert_eq!(load_settings_at(&path), AppSettings::default());

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn default_location_sits_under_the_user_config() {
    let path = settings_path().expect("系统应该有配置目录");
    assert!(
        path.ends_with("lumen-frame/settings.toml"),
        "{}",
        path.display()
    );
    // 和预设分开放：预设是一份画面效果，这里是这台机器的应用偏好。
    assert!(!path.to_string_lossy().contains("presets"));
}
