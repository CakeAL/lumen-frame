//! 自定义素材库、透明图像与相邻组共用预览/导出管线。
use lumen_frame::{
    media::{ExifInfo, ensure_vips, image_write_to_memory},
    persistence::{
        logos,
        presets::{WatermarkPreset, load_in, save_in},
    },
    watermark::{GroupAttachment, Placement, Text, TextGroup, TextParams, WatermarkParams},
};
use std::{collections::BTreeMap, sync::Arc};

const SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><path d="M10 5H70V35H10Z M35 15V25H45V15Z" fill="#ffca00" fill-rule="evenodd"/></svg>"##;

fn logo_group(token: &str) -> TextGroup {
    TextGroup {
        text: Text {
            template: vec![token.into()],
            text_params: vec![TextParams {
                size: 0.08,
                ..Default::default()
            }],
        },
        ..Default::default()
    }
}

#[test]
fn imported_custom_logos_keep_numbering_and_survive_source_removal_and_presets() {
    let root = std::env::temp_dir().join(format!("lumen-frame-logos-{}", std::process::id()));
    let source = root.join("source.svg");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(&source, SVG).unwrap();
    let support = root.join("support");
    let first = logos::import_in(&support, &[source.clone()]).unwrap();
    let second = logos::import_in(&support, &[source.clone()]).unwrap();
    assert_eq!(first[0].token(), "{自定义logo1}");
    assert_eq!(second[1].token(), "{自定义logo2}");
    std::fs::remove_file(source).unwrap();
    let loaded = logos::load_in(&support).unwrap();
    assert_eq!(*loaded[0].bytes().as_ref(), SVG);
    let mut group = logo_group("{自定义logo1}");
    group.text.text_params[0].logo_size = Some(0.11);
    group.attachment = Some(GroupAttachment {
        target: 1,
        side: Placement::Left,
        gap: 0.0,
    });
    let params = WatermarkParams {
        custom_logos: logos::snapshot(&loaded),
        ..Default::default()
    };
    let preset = WatermarkPreset {
        params,
        text_groups: vec![group, TextGroup::default()],
    };
    let path = save_in(&root.join("presets"), "logo", &preset).unwrap();
    let document = std::fs::read_to_string(path).unwrap();
    assert!(!document.contains("custom_logos"));
    assert!(!document.contains("support"));
    let restored = load_in(&root.join("presets"), "logo").unwrap();
    assert_eq!(restored.text_groups, preset.text_groups);
    assert!(restored.params.custom_logos.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn transparent_png_and_svg_render_in_logo_only_groups_and_size_scales() {
    ensure_vips();
    let mut image = image::RgbaImage::new(80, 40);
    for y in 5..35 {
        for x in 10..70 {
            if (35..45).contains(&x) && (15..25).contains(&y) {
                continue;
            }
            image.put_pixel(x, y, image::Rgba([255, 202, 0, 128]));
        }
    }
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).unwrap();
    for (is_png, bytes) in [(false, SVG.to_vec()), (true, png.into_inner())] {
        let params = WatermarkParams {
            custom_logos: BTreeMap::from([("自定义logo1".into(), Arc::new(bytes))]),
            ..Default::default()
        };
        let mut group = logo_group("{自定义logo1}");
        let small = group
            .render_text(&ExifInfo::default(), 1000, &params)
            .unwrap()
            .unwrap();
        let pixels = image_write_to_memory(&small).unwrap();
        // 大小针对可见内容；素材和字体行框的上下空白都不能撑高这一行。
        assert_eq!(small.height(), 80);
        assert!((small.width() as i32 - 160).abs() <= 2);
        assert!(pixels.chunks_exact(4).any(|p| p[3] == 0));
        assert!(
            pixels
                .chunks_exact(4)
                .any(|p| p[3] > 0 && p[0] > 240 && p[1] > 180 && p[2] < 20)
        );
        group.text.text_params[0].size *= 2.0;
        let big = group
            .render_text(&ExifInfo::default(), 1000, &params)
            .unwrap()
            .unwrap();
        assert!((big.width() as i64 - 2 * small.width() as i64).abs() <= 4);
        assert_eq!(big.height(), 160);
        if is_png {
            // 缩放插值允许边缘有少量振铃，但不能把半透明素材变成不透明。
            assert!(pixels.chunks_exact(4).any(|p| p[3] == 128));
            assert!(!pixels.chunks_exact(4).any(|p| p[3] > 160));
        }
    }
}

#[test]
fn logo_height_changes_without_resizing_the_separator_or_adding_outer_line_spacing() {
    let params = WatermarkParams {
        custom_logos: BTreeMap::from([("自定义logo1".into(), Arc::new(SVG.to_vec()))]),
        ..Default::default()
    };
    let mut group = logo_group("  {自定义logo1} | {自定义logo1}");
    let line = &mut group.text.text_params[0];
    line.size = 0.025;
    line.color = Some([255, 255, 255]);
    line.logo_size = Some(0.08);
    let small = group
        .render_text(&ExifInfo::default(), 1000, &params)
        .unwrap()
        .unwrap();
    group.text.text_params[0].logo_size = Some(0.12);
    group.text.text_params[0].line_spacing = 2.5;
    let large = group
        .render_text(&ExifInfo::default(), 1000, &params)
        .unwrap()
        .unwrap();
    // 分隔符的字形下降部可以略低于 Logo，但不能多出一整段字体行框。
    assert!((80..=90).contains(&small.height()));
    assert!((120..=130).contains(&large.height()));
    let logo_height = |image: &vips::VipsImage<'static>| {
        let pixels = image_write_to_memory(image).unwrap();
        let rows = pixels
            .chunks_exact(image.width() as usize * 4)
            .enumerate()
            .filter(|(_, row)| {
                row.chunks_exact(4)
                    .any(|p| p[3] != 0 && p[0] > 240 && p[1] > 180 && p[2] < 20)
            })
            .map(|(y, _)| y)
            .collect::<Vec<_>>();
        rows.last().unwrap() - rows.first().unwrap() + 1
    };
    assert_eq!(logo_height(&small), 80);
    assert_eq!(logo_height(&large), 120);

    let separator_size = |image: &vips::VipsImage<'static>| {
        let pixels = image_write_to_memory(image).unwrap();
        let mut bounds = (i32::MAX, i32::MAX, 0, 0);
        for (ix, p) in pixels.chunks_exact(4).enumerate() {
            if p[3] != 0 && p[0] == 255 && p[1] == 255 && p[2] == 255 {
                let (x, y) = (
                    ix as i32 % image.width() as i32,
                    ix as i32 / image.width() as i32,
                );
                bounds.0 = bounds.0.min(x);
                bounds.1 = bounds.1.min(y);
                bounds.2 = bounds.2.max(x + 1);
                bounds.3 = bounds.3.max(y + 1);
            }
        }
        assert!(bounds.0 < bounds.2 && bounds.1 < bounds.3);
        (bounds.2 - bounds.0, bounds.3 - bounds.1)
    };
    assert_eq!(separator_size(&small), separator_size(&large));

    group.text.template = vec!["{自定义logo1}".into(); 2];
    group.text.text_params = vec![group.text.text_params[0].clone(); 2];
    let two_rows = group
        .render_text(&ExifInfo::default(), 1000, &params)
        .unwrap()
        .unwrap();
    // 两个 120px Logo，加上字号 25px × (2.5 - 1) 的显式行间距。
    assert_eq!(two_rows.height(), 120 * 2 + 38);

    // 旧预设没有 Logo 高度时仍可载入；纯 Logo 行不再被行距扩出上下空白。
    let legacy: TextParams = toml::from_str("size = 0.07").unwrap();
    assert_eq!(legacy.logo_size, None);
    group.text.template = vec!["{自定义logo1}".into()];
    group.text.text_params = vec![legacy];
    let legacy_image = group
        .render_text(&ExifInfo::default(), 1000, &params)
        .unwrap()
        .unwrap();
    assert_eq!(legacy_image.height(), 70);
}
