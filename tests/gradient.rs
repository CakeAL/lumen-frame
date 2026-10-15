//! 渐变背景通过完整合成与导出，并保留 Ultra HDR 的照片区域 gain map。
use lumen_frame::{
    features::photo_palette,
    gainmap,
    media::{image_write_to_memory, load_base_image},
    photo::Photo,
    watermark::{BackgroundGradient, GradientPalette, PaletteTheme, WatermarkParams},
};

#[test]
fn hdr_gradient_background_keeps_gainmap_and_corners() {
    let input = load_base_image(std::path::Path::new("test_images/ultra_hdr.jpg")).unwrap();
    assert!(gainmap::get_gainmap(&input).is_some());
    let params = WatermarkParams {
        gradient_background: true,
        gradient: "linear-gradient(135deg,#f02fc2,#6094ea)".parse().unwrap(),
        shadow_size: 0.0,
        ..Default::default()
    };
    let image = Photo::compose_watermark(input, None, &params, &[]).unwrap();
    let map = gainmap::get_gainmap(&image).expect("渐变不能使 Ultra HDR 降为 SDR");
    let map_pixels = image_write_to_memory(&map).unwrap();
    assert_eq!(map_pixels[0], 0, "背景 gain map 必须保持中性");
    assert!(map_pixels.iter().any(|&v| v > 0));
    let pixels = image_write_to_memory(&image).unwrap();
    assert_eq!(&pixels[..4], &[0xf0, 0x2f, 0xc2, 255]);
    let end = pixels.len();
    assert_eq!(&pixels[end - 4..], &[0x60, 0x94, 0xea, 255]);
}

#[test]
fn palette_recipe_uses_the_target_photo_in_preview_and_hdr_export() {
    use lumen_frame::ui::{PreviewJob, render_preview};
    use std::{
        path::{Path, PathBuf},
        time::SystemTime,
    };
    let source_colors = photo_palette::extract_with_theme(
        Path::new("test_images/DSC_4587.jpg"),
        6,
        PaletteTheme::Muted,
    )
    .unwrap();
    let target = PathBuf::from("test_images/ultra_hdr.jpg");
    let target_colors = photo_palette::extract_with_theme(&target, 6, PaletteTheme::Muted).unwrap();
    let output = std::env::temp_dir().join(format!(
        "lumen-gradient-dynamic-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let params = WatermarkParams {
        gradient_background: true,
        gradient: BackgroundGradient::new(
            135.0,
            vec![
                source_colors[0].rgb(),
                source_colors[1.min(source_colors.len() - 1)].rgb(),
            ],
        )
        .unwrap(),
        gradient_palette: GradientPalette::new(PaletteTheme::Muted, vec![Some(0), Some(1)]),
        shadow_size: 0.0,
        output_folder: Some(output.clone()),
        ..Default::default()
    };
    assert_ne!(source_colors[0].rgb(), target_colors[0].rgb());
    let photo = Photo::open_blocking(&target).unwrap();
    let composed = photo.generate_watermark(&params, &[]).unwrap();
    let bytes = image_write_to_memory(&composed).unwrap();
    let rgb = target_colors[0].rgb();
    assert_eq!(&bytes[..3], &rgb);
    let preview = render_preview(&PreviewJob {
        path: target.clone(),
        exif: None,
        params: params.clone(),
        text_groups: vec![],
        max_edge: 320,
    })
    .unwrap();
    assert_eq!(
        &preview.as_bytes(0).unwrap()[..4],
        &[rgb[2], rgb[1], rgb[0], 255]
    );
    photo.save_image(&params, &composed).unwrap();
    let exported = load_base_image(&output.join("ultra_hdr_watermark.jpg")).unwrap();
    let map = gainmap::get_gainmap(&exported).expect("动态色卡渐变导出不能丢失 gain map");
    assert_eq!(image_write_to_memory(&map).unwrap()[0], 0);
    std::fs::remove_dir_all(output).unwrap();
}
