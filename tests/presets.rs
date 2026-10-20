//! 预设的保存与载入。
//!
//! 预设是这套界面里唯一会落盘的东西，所以它的契约值得钉死：存进去什么、拿回来什么、
//! 什么不进文件、以及名字里的路径分隔符不会写到目录外面去。

use std::path::PathBuf;

use lumen_frame_core::{
    persistence::presets::{
        WatermarkPreset, builtin, delete_in, directory, list_in, load_in, save_in,
    },
    watermark::{
        Placement, Text, TextAlign, TextDirection, TextGroup, TextParams, TextSpan, TextStyle,
        WatermarkParams,
    },
};

/// 每个用例一个独立目录，互不干扰。
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lumen-frame-presets-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn colourful_preset() -> WatermarkPreset {
    WatermarkPreset {
        params: WatermarkParams {
            border_ratio: (0.02, 0.03, 0.04, 0.05),
            aspect_ratio: Some((5.0, 3.0)),
            position: Placement::Up,
            background: [18, 52, 86],
            solid_background: true,
            border_radius: 0.035,
            shadow_size: 0.07,
            shadow_color: [255; 3],
            blur_sigma: 88.0,
            background_grain: true,
            grain_strength: 0.35,
            quality: 82,
            output_folder: Some(PathBuf::from("/should/not/be/saved")),
            default_font: "Menlo".to_owned(),
            ..Default::default()
        },
        text_groups: vec![TextGroup {
            text: Text {
                template: vec!["{Logo} {型号}".to_owned(), "自定义一行".to_owned()],
                text_params: vec![
                    TextParams {
                        size: 0.026,
                        bold: true,
                        align: TextAlign::Right,
                        ..Default::default()
                    },
                    TextParams {
                        size: 0.018,
                        italic: true,
                        ..Default::default()
                    },
                ],
            },
            position: Placement::Center,
            direction: TextDirection::Vertical,
            align: TextAlign::Right,
            padding: 0.04,
            photo_placement: None,
            time_format: "%Y年%m月%d日".to_owned(),
            attachment: None,
            border: Default::default(),
            background: None,
        }],
    }
}

#[test]
fn preset_round_trips_through_a_file() {
    let dir = scratch_dir("round-trip");
    let mut original = colourful_preset();
    let mut inside = lumen_frame_core::watermark::PhotoPlacement::new(
        lumen_frame_core::watermark::PhotoAnchor::TopRight,
    );
    inside.set_inset(0.027);
    original.text_groups[0].photo_placement = Some(inside);
    original.params.photo_border.set_enabled(true);
    original.params.photo_border.set_width(0.017);
    original.params.palette_card.set_enabled(true);
    original.params.palette_card.set_selected(vec![4, 1, 2]);
    original
        .params
        .palette_card
        .set_shape(lumen_frame_core::watermark::PaletteShape::Rounded);
    original.params.palette_card.set_proportional(true);
    original.params.palette_card.set_show_hex(true);
    original.params.palette_card.set_show_ratio(true);
    original
        .params
        .palette_card
        .set_direction(TextDirection::Vertical);
    original
        .params
        .palette_card
        .set_theme(lumen_frame_core::watermark::PaletteTheme::Vivid);
    original.params.palette_card.set_size(0.04);
    original.params.palette_card.set_gap(0.003);
    original.params.palette_card.set_position(Placement::Left);
    original.params.palette_card.set_attachment(Some(
        lumen_frame_core::watermark::GroupAttachment {
            target: 0,
            side: Placement::Right,
            gap: 0.015,
        },
    ));
    original.text_groups[0].background = Some([12, 34, 56, 128]);
    original.text_groups[0].border.set_enabled(true);
    original.text_groups[0].border.set_width(0.0012);
    original.text_groups[0].border.set_radius(0.006);
    original.text_groups[0].border.set_inset(0.008);
    original.text_groups[0].border.set_color([160, 165, 170]);
    original.text_groups[0].border.set_min_width(0.1);
    original.text_groups[0].border.set_min_height(0.05);
    original.text_groups[0].text.text_params[0].border =
        Some(original.text_groups[0].border.clone());
    original.text_groups[0]
        .border
        .set_side(Placement::Up, false);
    original
        .params
        .photo_border
        .select_palette(2, [42, 82, 125]);
    original.text_groups[0].text.text_params[0].spans = vec![TextSpan {
        start: 7,
        end: 15,
        style: TextStyle {
            font: Some("Menlo".into()),
            color: Some([220, 40, 80]),
            ..Default::default()
        },
    }];

    let path = save_in(&dir, "旅拍 5:3", &original).expect("保存失败");
    assert!(path.exists(), "保存后文件应该存在：{}", path.display());

    let loaded = load_in(&dir, "旅拍 5:3").expect("载入失败");

    assert_eq!(loaded.params.border_ratio, original.params.border_ratio);
    assert_eq!(loaded.params.aspect_ratio, original.params.aspect_ratio);
    assert_eq!(loaded.params.background, original.params.background);
    assert_eq!(loaded.params.shadow_color, original.params.shadow_color);
    assert_eq!(loaded.params.photo_border, original.params.photo_border);
    assert_eq!(loaded.params.palette_card, original.params.palette_card);
    assert_eq!(loaded.params.quality, WatermarkParams::default().quality);
    assert_eq!(loaded.params.blur_sigma, original.params.blur_sigma);
    assert_eq!(
        loaded.params.background_grain,
        original.params.background_grain
    );
    assert_eq!(loaded.params.grain_strength, original.params.grain_strength);
    assert_eq!(loaded.text_groups, original.text_groups);
    assert_eq!(loaded.text_groups[0].position, Placement::Center);
    assert_eq!(loaded.text_groups[0].direction, TextDirection::Vertical);
    assert_eq!(loaded.text_groups[0].align, TextAlign::Right);
    assert_eq!(
        loaded.text_groups[0].text.text_params[0].align,
        TextAlign::Right
    );
    assert!(loaded.text_groups[0].text.text_params[1].italic);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn gradient_presets_roundtrip_and_old_backgrounds_keep_their_mode() {
    use lumen_frame_core::watermark::{
        BackgroundGradient, BackgroundKind, GradientPalette, PaletteTheme,
    };
    let dir = scratch_dir("gradient");
    let mut preset = colourful_preset();
    preset.params.gradient = "linear-gradient(135deg,#f02fc2,#6094ea,#fff)"
        .parse::<BackgroundGradient>()
        .unwrap();
    preset.params.set_background_kind(BackgroundKind::Gradient);
    preset.params.gradient_palette =
        GradientPalette::new(PaletteTheme::Vivid, vec![None, Some(0), Some(2)]);
    save_in(&dir, "多色渐变", &preset).unwrap();
    let loaded = load_in(&dir, "多色渐变").unwrap();
    assert_eq!(loaded.params.background_kind(), BackgroundKind::Gradient);
    assert_eq!(loaded.params.gradient, preset.params.gradient);
    assert_eq!(
        loaded.params.gradient_palette,
        preset.params.gradient_palette
    );
    let old: WatermarkParams =
        toml::from_str("solid_background = true\nbackground = [18,52,86]").unwrap();
    assert_eq!(old.background_kind(), BackgroundKind::Solid);
    assert_eq!(old.shadow_color, [0; 3]);
    assert!(!old.palette_card.enabled());
    assert!(!old.background_grain);
    assert_eq!(old.grain_strength, 0.2);
    assert_eq!(
        WatermarkParams::default().background_kind(),
        BackgroundKind::Photo
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn output_folder_is_not_part_of_a_preset() {
    let dir = scratch_dir("output-folder");
    let preset = colourful_preset();

    let path = save_in(&dir, "no-folder", &preset).unwrap();
    let document = std::fs::read_to_string(&path).unwrap();
    assert!(
        !document.contains("output_folder")
            && !document.contains("should/not/be/saved")
            && !document.contains("version =")
            && !document.contains("default_font")
            && !document.contains("quality"),
        "本机环境设置不该写进预设：\n{document}"
    );

    // 载入回来的是结构体默认值，所以载入方必须显式保留当前值。
    // `AppView::apply_preset` 就是这么做的。
    let loaded = load_in(&dir, "no-folder").unwrap();
    assert_ne!(loaded.params.output_folder, preset.params.output_folder);
    assert_ne!(loaded.params.default_font, preset.params.default_font);
    assert_ne!(loaded.params.quality, preset.params.quality);

    // 旧预设仍可载入，其中的质量值不能覆盖全局导出偏好。
    let legacy = document.replace("[params]", "[params]\nquality = 17");
    std::fs::write(&path, legacy).unwrap();
    let loaded = load_in(&dir, "no-folder").unwrap();
    assert_eq!(loaded.params.quality, WatermarkParams::default().quality);
    assert_eq!(loaded.params.border_ratio, preset.params.border_ratio);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn solid_palette_recipe_survives_saving_and_follows_each_photo() {
    use lumen_frame_core::{Session, photo_palette, watermark::GradientPalette};
    let dir = scratch_dir("solid-palette");
    let mut preset = colourful_preset();
    preset.params.background_palette_rank = Some(0);
    preset.params.gradient = "linear-gradient(135deg,#a44017,#20140c,#73270d)"
        .parse()
        .unwrap();
    preset.params.gradient_palette =
        GradientPalette::new(Default::default(), vec![Some(0), Some(1), Some(2)]);
    let path = save_in(&dir, "大面积色卡", &preset).unwrap();
    let document: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let params = &document["params"];
    assert_eq!(params["background_palette_rank"].as_integer(), Some(0));
    assert!(params.get("gradient_background").is_none());
    assert!(params.get("gradient").is_none());
    assert!(params["gradient_palette"].get("slots").is_none());
    let loaded = load_in(&dir, "大面积色卡").unwrap();
    assert_eq!(loaded.params.background_palette_rank, Some(0));
    let mut session = Session::default();
    session.add_photos(vec![
        "test_images/DSC_4587.jpg".into(),
        "test_images/ultra_hdr.jpg".into(),
    ]);
    session.apply_preset(loaded, true);
    let mut colors = Vec::new();
    for photo in session.workspace().photos() {
        let params = session.watermark_for(photo.id()).unwrap().params;
        assert_eq!(params.background_palette_rank, Some(0));
        let expected =
            photo_palette::extract_with_theme(photo.path(), 6, params.gradient_palette.theme())
                .unwrap()[0]
                .rgb();
        let resolved = photo_palette::resolve(photo.path(), &params).unwrap();
        assert_eq!(resolved.background, expected);
        colors.push(expected);
    }
    assert_ne!(colors[0], colors[1], "应用同一预设应使用各自照片的第一色");
    preset.params.background_palette_rank = None;
    let manual_path = save_in(&dir, "固定颜色", &preset).unwrap();
    let manual = load_in(&dir, "固定颜色").unwrap();
    assert!(manual.params.background_palette_rank.is_none());
    assert_eq!(
        photo_palette::resolve(std::path::Path::new("不存在的照片"), &manual.params)
            .unwrap()
            .background,
        preset.params.background
    );
    assert!(
        !std::fs::read_to_string(manual_path)
            .unwrap()
            .contains("background_palette_rank")
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn custom_photo_content_is_not_written_into_a_reusable_preset() {
    let dir = scratch_dir("custom-photo-text");
    let mut preset = colourful_preset();
    preset.params.custom_text = "只属于这一张照片".into();
    preset.text_groups[0].text.template[0] = "{Logo} {自定义文本}".into();
    let path = save_in(&dir, "custom-field", &preset).unwrap();
    let document = std::fs::read_to_string(path).unwrap();
    assert!(!document.contains("custom_text"));
    assert!(!document.contains("只属于这一张照片"));
    assert!(document.contains("{自定义文本}"));
    let loaded = load_in(&dir, "custom-field").unwrap();
    assert!(loaded.params.custom_text.is_empty());
    assert_eq!(loaded.text_groups, preset.text_groups);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn palette_only_presets_keep_text_groups_empty_when_applied() {
    use lumen_frame_core::Session;
    let dir = scratch_dir("palette-only");
    let mut preset = colourful_preset();
    preset.text_groups.clear();
    preset.params.palette_card.set_enabled(true);
    save_in(&dir, "色卡", &preset).unwrap();
    let loaded = load_in(&dir, "色卡").unwrap();
    assert!(loaded.text_groups.is_empty());

    for all in [false, true] {
        let mut session = Session::default();
        session.add_photos(vec![
            "test_images/DSC_4587.jpg".into(),
            "test_images/ultra_hdr.jpg".into(),
        ]);
        session.apply_preset(loaded.clone(), all);
        assert!(session.current().text_groups.is_empty());
        assert!(session.params().palette_card.enabled());
        for photo in session.workspace().photos() {
            let watermark = session.watermark_for(photo.id()).unwrap();
            let should_be_empty = all || Some(photo.id()) == session.workspace().selected_id();
            assert_eq!(watermark.text_groups.is_empty(), should_be_empty);
        }
    }
    // 空队列加载同一预设后，新导入的照片也不应恢复默认文字。
    let mut session = Session::default();
    session.apply_preset(loaded, true);
    session.add_photos(vec!["test_images/DSC_4587.jpg".into()]);
    assert!(session.current().text_groups.is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn bundled_presets_load_from_assets() {
    let presets = builtin().expect("内置预设应该随应用一起可用");
    let names = presets
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        [
            "16_9",
            "基础样式",
            "文字在上",
            "白色边框",
            "纯色背景_文字在下",
            "百分比色卡&自定义尼康logo",
            "色卡",
            "文字在图片内部",
            "文字组背景色",
            "右侧参数卡_文字加框",
            "左侧参数卡_文字加框背景颗粒",
            "colorwalk_背景色卡",
        ]
    );
    assert!(
        presets
            .iter()
            .all(|(_, preset)| !preset.text_groups.is_empty())
    );
    assert!(
        presets
            .iter()
            // 新示例继承用户字体；已有预设允许保留显式的字体选择。
            .filter(|(name, _)| [
                "右侧参数卡_文字加框",
                "左侧参数卡_文字加框背景颗粒",
                "文字组背景色"
            ]
            .contains(&name.as_str()))
            .flat_map(|(_, preset)| &preset.text_groups)
            .all(|group| {
                group
                    .text
                    .text_params
                    .iter()
                    .all(|line| line.font.is_empty())
            })
    );
    let background = lumen_frame_core::persistence::presets::load_builtin("文字组背景色").unwrap();
    assert!(lumen_frame_core::persistence::presets::is_builtin(
        "文字组背景色"
    ));
    assert_eq!(background.text_groups.len(), 2);
    assert_eq!(
        background.text_groups[0].background,
        Some([254, 225, 0, 204])
    );
    assert_eq!(
        background.text_groups[1].background,
        Some([255, 255, 255, 204])
    );
    assert!(background.text_groups.iter().all(|group| {
        !group.border.enabled()
            && group.photo_placement.is_some()
            && group
                .text
                .text_params
                .iter()
                .all(|line| line.color.is_none())
    }));
    assert_eq!(
        background.text_groups[0]
            .attachment
            .as_ref()
            .unwrap()
            .target,
        1
    );
}

#[test]
fn inherited_and_explicit_fonts_round_trip() {
    let dir = scratch_dir("fonts");
    let mut preset = colourful_preset();
    preset.text_groups[0].text.text_params[1].font = "Menlo".to_owned();
    let path = save_in(&dir, "fonts", &preset).unwrap();
    let document = std::fs::read_to_string(&path).unwrap();
    assert_eq!(document.matches("font =").count(), 1);

    let loaded = load_in(&dir, "fonts").unwrap();
    assert!(loaded.text_groups[0].text.text_params[0].font.is_empty());
    assert_eq!(loaded.text_groups[0].text.text_params[1].font, "Menlo");

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn presets_are_listed_and_deleted() {
    let dir = scratch_dir("list-delete");
    let preset = colourful_preset();

    assert!(list_in(&dir).unwrap().is_empty());

    save_in(&dir, "乙", &preset).unwrap();
    save_in(&dir, "甲", &preset).unwrap();
    // 同名覆盖，不该变成两条。
    save_in(&dir, "甲", &preset).unwrap();

    // 同名覆盖不该变成两条；列表按名字排序。
    let names: Vec<String> = list_in(&dir).unwrap();
    assert_eq!(names.len(), 2, "同名保存应该覆盖而不是追加：{names:?}");
    let mut expected = names.clone();
    expected.sort();
    assert_eq!(names, expected, "预设列表应该按名字排序");

    delete_in(&dir, "甲").unwrap();
    assert_eq!(list_in(&dir).unwrap(), vec!["乙".to_string()]);

    // 再删一次不是错误：目标状态已经达成。
    delete_in(&dir, "甲").unwrap();

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn names_cannot_escape_the_preset_directory() {
    let dir = scratch_dir("escape");
    let preset = colourful_preset();

    let path = save_in(&dir, "../../escape", &preset).unwrap();
    assert_eq!(
        path.parent(),
        Some(dir.as_path()),
        "预设文件必须留在预设目录里：{}",
        path.display()
    );

    // 空名字和纯标点名字会被拒绝，而不是写出一个奇怪的文件。
    assert!(save_in(&dir, "   ", &preset).is_err());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn default_directory_sits_under_the_user_config() {
    let dir = directory().expect("系统应该有配置目录");
    assert!(dir.ends_with("lumen-frame/presets"), "{}", dir.display());
}

#[test]
fn legacy_text_groups_have_no_background_and_zero_alpha_roundtrips() {
    let mut preset = colourful_preset();
    let legacy = toml::to_string(&preset).unwrap();
    let loaded: WatermarkPreset = toml::from_str(&legacy).unwrap();
    assert_eq!(loaded.text_groups[0].background, None);
    for alpha in [0, 1, 128, 255] {
        preset.text_groups[0].background = Some([10, 20, 30, alpha]);
        let loaded: WatermarkPreset = toml::from_str(&toml::to_string(&preset).unwrap()).unwrap();
        assert_eq!(
            loaded.text_groups[0].background,
            preset.text_groups[0].background
        );
    }
}
