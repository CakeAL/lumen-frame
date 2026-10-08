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

const SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><rect x="10" y="5" width="60" height="30" fill="#ffca00"/></svg>"##;

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
            image.put_pixel(x, y, image::Rgba([255, 202, 0, 128]));
        }
    }
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).unwrap();
    for bytes in [SVG.to_vec(), png.into_inner()] {
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
    }
}
