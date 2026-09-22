//! 预设的保存与载入。
//!
//! 预设是这套界面里唯一会落盘的东西，所以它的契约值得钉死：存进去什么、拿回来什么、
//! 什么不进文件、以及名字里的路径分隔符不会写到目录外面去。

use std::path::PathBuf;

use lumen_frame::{
    persistence::presets::{
        WatermarkPreset, builtin, delete_in, directory, list_in, load_in, save_in,
    },
    watermark::{
        Placement, Text, TextAlign, TextDirection, TextGroup, TextParams, WatermarkParams,
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
            blur_sigma: 88.0,
            quality: 82,
            output_folder: Some(PathBuf::from("/should/not/be/saved")),
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
            time_format: "%Y年%m月%d日".to_owned(),
        }],
    }
}

#[test]
fn preset_round_trips_through_a_file() {
    let dir = scratch_dir("round-trip");
    let original = colourful_preset();

    let path = save_in(&dir, "旅拍 5:3", &original).expect("保存失败");
    assert!(path.exists(), "保存后文件应该存在：{}", path.display());

    let loaded = load_in(&dir, "旅拍 5:3").expect("载入失败");

    assert_eq!(loaded.params.border_ratio, original.params.border_ratio);
    assert_eq!(loaded.params.aspect_ratio, original.params.aspect_ratio);
    assert_eq!(loaded.params.background, original.params.background);
    assert_eq!(loaded.params.quality, original.params.quality);
    assert_eq!(loaded.params.blur_sigma, original.params.blur_sigma);
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
fn output_folder_is_not_part_of_a_preset() {
    let dir = scratch_dir("output-folder");
    let preset = colourful_preset();

    let path = save_in(&dir, "no-folder", &preset).unwrap();
    let document = std::fs::read_to_string(&path).unwrap();
    assert!(
        !document.contains("output_folder")
            && !document.contains("should/not/be/saved")
            && !document.contains("version ="),
        "输出文件夹属于本机环境，不该写进预设：\n{document}"
    );

    // 载入回来的是结构体默认值，所以载入方必须显式保留当前值。
    // `AppView::apply_preset` 就是这么做的。
    let loaded = load_in(&dir, "no-folder").unwrap();
    assert_ne!(loaded.params.output_folder, preset.params.output_folder);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bundled_presets_load_from_assets() {
    let presets = builtin().expect("内置预设应该随应用一起可用");
    let names = presets
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(presets.len(), 5);
    assert!(names.contains(&"基础样式"));
    assert!(names.contains(&"纯色背景_文字在下"));
    assert!(
        presets
            .iter()
            .all(|(_, preset)| !preset.text_groups.is_empty())
    );
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
