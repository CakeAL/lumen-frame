//! 应用级偏好（目前是界面明暗）的保存与读取。
//!
//! 这份文件不该有能力拦住启动：读不到、读坏了都退回默认值。

use std::path::PathBuf;

use lumen_frame_core::persistence::settings::{
    AppSettings, AppearanceMode, load_at, save_at, settings_path,
};
use lumen_frame_core::watermark::{DEFAULT_FONT, DEFAULT_JPEG_QUALITY};

fn scratch_file(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lumen-frame-settings-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("settings.toml")
}

#[test]
fn default_appearance_follows_the_system() {
    // 新装的应用没有偏好文件，这时应该跟随系统，而不是自作主张选一个明暗。
    assert_eq!(AppSettings::default().appearance, AppearanceMode::System);
    assert!(AppSettings::default().window_effects);
    assert_eq!(AppSettings::default().default_font, DEFAULT_FONT);
    assert_eq!(AppSettings::default().jpeg_quality, DEFAULT_JPEG_QUALITY);
    assert_eq!(
        AppSettings::default().preview_background,
        [0x9a, 0xa7, 0xb1]
    );
    assert_eq!(load_at(&scratch_file("missing")), AppSettings::default());
}

#[test]
fn appearance_round_trips() {
    let path = scratch_file("round-trip");

    for mode in [
        AppearanceMode::Dark,
        AppearanceMode::Light,
        AppearanceMode::System,
    ] {
        save_at(
            &path,
            &AppSettings {
                appearance: mode,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(load_at(&path).appearance, mode, "{mode:?} 没能往返");
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
        window_effects: false,
        light_theme: Some("Catppuccin Latte".to_string()),
        dark_theme: Some("Catppuccin Mocha".to_string()),
        interface_scale: Some(18.0),
        preview_max_edge: Some(800),
        preview_background: [0x24, 0x32, 0x4a],
        output_folder: Some(PathBuf::from("/tmp/lumen-frame-output")),
        jpeg_quality: 81,
        default_font: "Menlo".to_owned(),
        builtin_presets_collapsed: true,
        preset_order: vec!["我的预设二".to_owned(), "我的预设一".to_owned()],
        ..Default::default()
    };
    save_at(&path, &settings).unwrap();
    assert_eq!(load_at(&path), settings);

    // 没选过主题时是 None，表示用默认的那套，而不是空字符串。
    save_at(&path, &AppSettings::default()).unwrap();
    let loaded = load_at(&path);
    assert_eq!(loaded.light_theme, None);
    assert_eq!(loaded.dark_theme, None);
    assert_eq!(
        loaded.interface_scale, None,
        "没调过缩放时应为 None，而不是某个数值"
    );
    assert_eq!(loaded.preview_background, [0x9a, 0xa7, 0xb1]);
    assert_eq!(loaded.preview_max_edge, None);
    assert_eq!(loaded.default_font, DEFAULT_FONT);
    assert_eq!(loaded.jpeg_quality, DEFAULT_JPEG_QUALITY);
    assert!(!loaded.builtin_presets_collapsed);
    assert!(loaded.preset_order.is_empty());

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn old_settings_without_a_font_use_the_default() {
    let path = scratch_file("old-settings");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "appearance = \"Dark\"\n").unwrap();

    let loaded = load_at(&path);
    assert_eq!(loaded.appearance, AppearanceMode::Dark);
    assert!(
        loaded.window_effects,
        "旧配置未指定时应默认启用窗口背景效果"
    );
    assert_eq!(loaded.default_font, DEFAULT_FONT);
    assert_eq!(loaded.jpeg_quality, DEFAULT_JPEG_QUALITY);
    assert!(!loaded.builtin_presets_collapsed);
    assert!(loaded.preset_order.is_empty());

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_broken_file_falls_back_instead_of_failing_to_start() {
    let path = scratch_file("broken");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();

    std::fs::write(&path, "这不是 TOML {{{").unwrap();
    assert_eq!(load_at(&path), AppSettings::default());

    // 未知的取值同样退回默认，而不是让应用起不来。
    std::fs::write(&path, "appearance = \"Rainbow\"\n").unwrap();
    assert_eq!(load_at(&path), AppSettings::default());

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

#[test]
fn map_preferences_round_trip_and_old_settings_keep_defaults() {
    use lumen_frame_core::features::geolocation::{MapProvider, MapSettings};
    let path = scratch_file("map-source");
    for provider in [MapProvider::Tencent, MapProvider::OpenStreetMap] {
        let settings = AppSettings {
            map: MapSettings::default().with_provider(provider),
            ..Default::default()
        };
        save_at(&path, &settings).unwrap();
        assert_eq!(load_at(&path), settings);
    }
    std::fs::write(&path, "appearance = \"Dark\"\n").unwrap();
    let old = load_at(&path);
    assert_eq!(old.appearance, AppearanceMode::Dark);
    assert_eq!(old.map, MapSettings::default());
    // 移除的来源只回退地图选择，不应连带丢失其他应用偏好。
    std::fs::write(&path, "appearance = \"Dark\"\n[map]\nprovider = \"Custom\"\ncustom_url = \"https://example.com\"\n").unwrap();
    assert_eq!(load_at(&path).appearance, AppearanceMode::Dark);
    assert_eq!(load_at(&path).map, MapSettings::default());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn geocoding_preferences_round_trip_without_entering_presets() {
    use lumen_frame_core::{
        Session, StoragePaths,
        geocoding::{AddressField, GeocodingProvider, GeocodingSettings},
        persistence::presets::WatermarkPreset,
        watermark::WatermarkParams,
    };
    let path = scratch_file("geocoding");
    let config = GeocodingSettings {
        provider: GeocodingProvider::Amap,
        api_key: "saved-test-key".into(),
        fields: vec![AddressField {
            id: 42,
            paths: "pois.0.name,addressComponent.township".into(),
        }],
    };
    let settings = AppSettings {
        geocoding: config.clone(),
        ..Default::default()
    };
    save_at(&path, &settings).unwrap();
    assert_eq!(load_at(&path).geocoding, config);
    let old: AppSettings = toml::from_str("appearance = 'System'").unwrap();
    assert_eq!(old.geocoding, GeocodingSettings::default());
    let session =
        Session::from_storage(StoragePaths::in_directory(path.parent().unwrap())).unwrap();
    assert_eq!(session.params().geocoding, config);
    let mut preset = WatermarkPreset {
        params: WatermarkParams {
            geocoding: config.clone(),
            ..Default::default()
        },
        text_groups: vec![],
    };
    let document = toml::to_string(&preset).unwrap();
    assert!(!document.contains("saved-test-key"));
    assert!(!document.contains("geocoding"));
    let mut session = session;
    preset.params.geocoding = GeocodingSettings::default();
    session.replace_current(preset);
    assert_eq!(session.params().geocoding, config);
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test_images");
    let added = session.add_photos(vec![
        fixtures.join("DSC_4587.jpg"),
        fixtures.join("ultra_hdr.jpg"),
    ]);
    let jobs = session.export_jobs(lumen_frame_core::export::ExportScope::AllPhotos);
    session.params_mut().geocoding.provider = GeocodingProvider::Builtin;
    session.params_mut().geocoding.api_key.clear();
    session.select(added[1].0);
    assert_eq!(
        session.params().geocoding.provider,
        GeocodingProvider::Builtin
    );
    assert!(session.params().geocoding.api_key.is_empty());
    assert!(
        jobs.iter()
            .all(|job| job.watermark.params.geocoding == config)
    );
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
