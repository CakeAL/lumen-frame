#[path = "support/fonts.rs"]
mod font_backend;

use chrono::Local;
use nom_exif::{Altitude, ExifDateTime, GPSInfo, URational};

use lumen_frame_core::{
    media::{ExifInfo, Rational},
    photo::Photo,
    render::{
        canvas::{self, Margin},
        text::{render_exif_template, render_watermark_template},
    },
    watermark::{
        Placement, Text, TextAlign, TextDirection, TextGroup, TextParams, WatermarkParams,
    },
};

#[test]
fn test_render_exif_template() {
    let template = "{拍摄日期} {等效焦距}mm f/{光圈} {快门}s ISO{ISO} {曝光补偿}EV";
    let exif_info = ExifInfo {
        created_time: Some(ExifDateTime::Aware(Local::now().into())),
        focal_length_in35mm_film: Some(50),
        exposure_time: Some(Rational::Fraction(1, 250)),
        f_number: Some(Rational::Fraction(18, 10)),
        exposure_bias_value: Some(Rational::Fraction(-1, 3)),
        focal_length: Some(Rational::Fraction(500, 10)),
        isospeed_ratings: Some(100),
        ..Default::default()
    };
    let text = render_exif_template(template, &exif_info, "%Y/%m/%d %H:%M:%S");
    dbg!(text);
}

#[test]
fn camera_logo_honors_line_and_token_colors_without_recoloring_custom_logos() {
    use lumen_frame_core::{
        media::{ensure_vips, image_write_to_memory},
        watermark::{TextSpan, TextStyle},
    };
    ensure_vips();
    let exif = ExifInfo {
        make: Some("NIKON CORPORATION".into()),
        ..Default::default()
    };
    let mut params = WatermarkParams {
        solid_background: true,
        background: [255; 3],
        ..Default::default()
    };
    params.custom_logos.insert("测试logo".into(), std::sync::Arc::new(
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#e02050"/></svg>"##.to_vec(),
    ));
    for (template, spans, expected) in [
        ("{Logo}", vec![], [255; 3]),
        (
            "{Logo}",
            vec![TextSpan {
                start: 0,
                end: 6,
                style: TextStyle {
                    color: Some([0; 3]),
                    ..Default::default()
                },
            }],
            [0; 3],
        ),
        ("{测试logo}", vec![], [224, 32, 80]),
    ] {
        let image = Text {
            template: vec![template.into()],
            text_params: vec![TextParams {
                color: Some([255; 3]),
                spans,
                ..Default::default()
            }],
        }
        .render_text(&exif, 1000, &params, "")
        .unwrap()
        .unwrap();
        let pixels = image_write_to_memory(&image).unwrap();
        let visible: Vec<_> = pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] >= 240)
            .collect();
        assert!(!visible.is_empty());
        assert!(
            visible.iter().all(|p| p[..3]
                .iter()
                .zip(expected)
                .all(|(c, expected)| c.abs_diff(expected) <= 20)),
            "{template} 的颜色应尊重其来源"
        );
    }
}

#[test]
fn line_borders_frame_values_without_enclosing_units() {
    use lumen_frame_core::{media::image_write_to_memory, watermark::TextBorder};
    let mut border = TextBorder::default();
    border.set_enabled(true);
    border.set_width(0.002);
    border.set_color([0, 120, 220]);
    border.set_min_width(0.12);
    border.set_min_height(0.045);
    let style = TextParams {
        size: 0.02,
        color: Some([0; 3]),
        border: Some(border),
        ..Default::default()
    };
    let params = WatermarkParams::default();
    for (value, unit) in [("16", "mm"), ("2.8", "f"), ("1/100", "s"), ("250", "ISO")] {
        let mut text = Text {
            template: vec![value.into()],
            text_params: vec![style.clone()],
        };
        let framed = text
            .render_text(&ExifInfo::default(), 1000, &params, "")
            .unwrap()
            .unwrap();
        assert_eq!((framed.width(), framed.height()), (120, 45));
        text.template.push(unit.into());
        text.text_params.push(TextParams {
            size: 0.014,
            color: Some([100; 3]),
            ..Default::default()
        });
        let labelled = text
            .render_text(&ExifInfo::default(), 1000, &params, "")
            .unwrap()
            .unwrap();
        assert_eq!(labelled.width(), 120);
        assert!(labelled.height() > 45);
        let pixels = image_write_to_memory(&labelled).unwrap();
        let mut frame_pixels = 0;
        let mut unit_pixels = 0;
        for (ix, pixel) in pixels.as_chunks::<4>().0.iter().enumerate() {
            let y = ix / 120;
            if pixel[0..3] == [0, 120, 220] && pixel[3] > 0 {
                frame_pixels += 1;
                assert!(y < 45, "边框不应包住下方的单位");
            }
            if pixel[0..3] == [100; 3] && pixel[3] > 0 {
                unit_pixels += 1;
                assert!(y > 45, "单位应在外框下方");
            }
        }
        assert!(frame_pixels > 0 && unit_pixels > 0);
    }
    let empty = Text {
        template: vec!["{型号}".into()],
        text_params: vec![style.clone()],
    };
    assert!(
        empty
            .render_text(&ExifInfo::default(), 1000, &params, "")
            .unwrap()
            .is_none()
    );
    let long = Text {
        template: vec!["1/100000000000000".into()],
        text_params: vec![style],
    };
    let long = long
        .render_text(&ExifInfo::default(), 1000, &params, "")
        .unwrap()
        .unwrap();
    assert!(long.width() > 120, "长文字应撑开最小宽度，不能裁字");
}

#[test]
fn text_group_border_keeps_each_side_independent_after_rotation() {
    use lumen_frame_core::media::image_write_to_memory;
    let params = WatermarkParams::default();
    let sides = [
        Placement::Up,
        Placement::Right,
        Placement::Bottom,
        Placement::Left,
    ];
    for direction in [TextDirection::Horizontal, TextDirection::Vertical] {
        let mut group = TextGroup {
            direction,
            ..Default::default()
        };
        group.text.template = vec!["50".into()];
        group.text.text_params.truncate(1);
        let plain = group
            .render_text(&ExifInfo::default(), 1000, &params)
            .unwrap()
            .unwrap();
        let plain_pixels = image_write_to_memory(&plain).unwrap();
        group.border.set_enabled(true);
        group.border.set_width(0.002);
        group.border.set_radius(0.01);
        group.border.set_color([190, 180, 170]);
        for mask in 0..16 {
            for (ix, side) in sides.iter().enumerate() {
                group.border.set_side(*side, mask & (1 << ix) != 0);
            }
            let image = group
                .render_text(&ExifInfo::default(), 1000, &params)
                .unwrap()
                .unwrap();
            if mask == 0 {
                assert_eq!(
                    (image.width(), image.height()),
                    (plain.width(), plain.height())
                );
                continue;
            }
            let (w, h) = (image.width() as usize, image.height() as usize);
            let offsets =
                std::array::from_fn::<_, 4, _>(|ix| if mask & (1 << ix) != 0 { 6 } else { 0 });
            assert_eq!(
                (w, h),
                (
                    plain.width() as usize + offsets[3] + offsets[1],
                    plain.height() as usize + offsets[0] + offsets[2]
                )
            );
            let pixels = image_write_to_memory(&image).unwrap();
            for (ix, (x, y)) in [(w / 2, 0), (w - 1, h / 2), (w / 2, h - 1), (0, h / 2)]
                .into_iter()
                .enumerate()
            {
                let pixel = &pixels[(y * w + x) * 4..(y * w + x) * 4 + 4];
                if mask & (1 << ix) != 0 {
                    assert_eq!(
                        pixel,
                        &[190, 180, 170, 255],
                        "{direction:?} mask={mask} side={ix}"
                    );
                } else {
                    let original_alpha = x
                        .checked_sub(offsets[3])
                        .zip(y.checked_sub(offsets[0]))
                        .filter(|&(x, y)| x < plain.width() as usize && y < plain.height() as usize)
                        .map(|(x, y)| plain_pixels[(y * plain.width() as usize + x) * 4 + 3])
                        .unwrap_or(0);
                    assert_eq!(pixel[3], original_alpha, "未启用的边只保留原文字");
                }
            }
            if mask == 15 {
                assert_eq!(pixels[3], 0, "圆角之外应透明");
            }
        }
    }
}

#[tokio::test]
async fn vertical_text_group_rotates_the_complete_group() {
    let photo = Photo::new("./test_images/DSC_4587.jpg").await.unwrap();
    let exif = photo.exif.as_ref().unwrap();
    let params = WatermarkParams::default();
    let text = Text {
        template: vec!["第一行".to_owned(), "第二行更长".to_owned()],
        text_params: vec![TextParams::default(), TextParams::default()],
    };
    let horizontal = TextGroup {
        text: text.clone(),
        ..TextGroup::default()
    }
    .render_text(exif, 1000, &params)
    .unwrap()
    .unwrap();
    let vertical = TextGroup {
        text,
        direction: TextDirection::Vertical,
        ..TextGroup::default()
    }
    .render_text(exif, 1000, &params)
    .unwrap()
    .unwrap();

    assert_eq!(vertical.width(), horizontal.height());
    assert_eq!(vertical.height(), horizontal.width());
}

#[tokio::test]
async fn side_padding_is_transparent_space_inside_the_text_image() {
    let photo = Photo::new("./test_images/DSC_4587.jpg").await.unwrap();
    let exif = photo.exif.as_ref().unwrap();
    let params = WatermarkParams::default();
    let plain = TextGroup {
        align: TextAlign::Left,
        ..TextGroup::default()
    }
    .render_text(exif, 1_000, &params)
    .unwrap()
    .unwrap();
    let padded = TextGroup {
        align: TextAlign::Left,
        padding: 0.05,
        ..TextGroup::default()
    }
    .render_text(exif, 1_000, &params)
    .unwrap()
    .unwrap();

    assert_eq!(padded.width(), plain.width() + 50);
    assert_eq!(padded.height(), plain.height());

    let vertical_plain = TextGroup {
        position: Placement::Left,
        align: TextAlign::Left,
        ..TextGroup::default()
    }
    .render_text(exif, 1_000, &params)
    .unwrap()
    .unwrap();
    let vertical_padded = TextGroup {
        position: Placement::Left,
        align: TextAlign::Left,
        padding: 0.05,
        ..TextGroup::default()
    }
    .render_text(exif, 1_000, &params)
    .unwrap()
    .unwrap();

    assert_eq!(vertical_padded.width(), vertical_plain.width());
    assert_eq!(vertical_padded.height(), vertical_plain.height() + 50);
}

#[test]
fn test_render_exif_template_missing_and_dedupe() {
    // 没有镜头型号、型号含有品牌前缀时：
    // 品牌 + 型号一起去重，缺失字段移除后仍保留用户输入的连接符。
    let exif_info = ExifInfo {
        make: Some("Xiaomi".to_owned()),
        model: Some("Xiaomi 15".to_owned()),
        lens_model: None,
        ..Default::default()
    };
    let t1 = render_exif_template("{品牌} {型号} - {镜头型号}", &exif_info, "%Y/%m/%d");
    assert_eq!(t1, "Xiaomi 15 -");
    let t2 = render_exif_template("{型号} - {镜头型号}", &exif_info, "%Y/%m/%d");
    assert_eq!(t2, "15 -");
}

#[test]
fn test_render_exif_template_gps_and_administrative_area() {
    let mut gps_info: GPSInfo = "+22.1643139-113.5570944/".parse().unwrap();
    gps_info.altitude = Altitude::BelowSeaLevel(URational::new(1234, 100));
    let exif_info = ExifInfo {
        gps_info: Some(gps_info),
        ..Default::default()
    };

    assert_eq!(
        render_exif_template("{GPS}", &exif_info, "%Y/%m/%d"),
        "22°9.86'N 113°33.43'W"
    );
    assert_eq!(
        render_exif_template("{海拔}", &exif_info, "%Y/%m/%d"),
        "-12.34"
    );

    let china_gps: GPSInfo = "+22.1643139+113.5570944/".parse().unwrap();
    let china_exif = ExifInfo {
        gps_info: Some(china_gps),
        ..Default::default()
    };
    let address = render_exif_template("{省} {市} {区}", &china_exif, "%Y/%m/%d");
    assert_eq!(address.split_whitespace().count(), 3);
}

#[tokio::test]
async fn test_render_text_with_logo_mixed() {
    let photo_path = "./test_images/DSC_4587.jpg";
    let output_path = "./test_images/watermark";
    let photo = Photo::new(photo_path).await.unwrap();
    let text = Text {
        template: vec![
            "{Logo} {型号} - {镜头型号} {品牌}".to_owned(),
            "{拍摄日期} {等效焦距}mm {实际焦距}mm f/{光圈} {快门}s ISO{ISO}".to_owned(),
        ],
        text_params: vec![
            TextParams {
                size: 0.03,
                italic: true,
                align: TextAlign::Left,
                bold: true,
                font: "Maple Mono NF CN".into(),
                ..Default::default()
            },
            TextParams {
                size: 0.022,
                align: TextAlign::Left,
                font: "Maple Mono NF CN".into(),
                ..Default::default()
            },
        ],
    };
    let params = WatermarkParams {
        output_folder: Some(output_path.into()),
        aspect_ratio: Some((16.0, 9.0)),
        position: Placement::Left,
        blur_sigma: 15.0,
        background: [26, 188, 156],
        border_radius: 0.02,
        solid_background: true,
        ..Default::default()
    };
    let img = lumen_frame_core::media::load_base_image(&photo.path).unwrap();
    let (img_w, img_h) = (img.width() as i32, img.height() as i32);
    let margin = Margin::cal_margin(img_w, img_h, &params);
    let (canvas_w, _canvas_h) = canvas::cal_size(&margin, img_w, img_h, &params);

    let text_img = text
        .render_text(photo.exif.as_ref().unwrap(), img_h, &params, "%Y/%m/%d")
        .unwrap()
        .expect("render_text should produce an image");

    let (w, h) = (text_img.width() as i32, text_img.height() as i32);
    assert!(w > 0, "text image width must be positive");
    assert!(h > 0, "text image height must be positive");
    // 已去掉左右空白：图片宽度 = 最长一行，而不是整个画布宽度
    assert!(
        w < canvas_w,
        "text image should be trimmed to the longest line width (w={w}, canvas_w={canvas_w})"
    );

    // 确保确实有可见像素（文字或 logo）
    let pixels = text_img.write_to_memory().unwrap();
    let has_visible = pixels.as_chunks::<4>().0.iter().any(|p| p[3] > 0);
    assert!(
        has_visible,
        "rendered text image should not be fully transparent"
    );

    std::fs::create_dir_all(output_path).unwrap();
    text_img
        .write_to_file(format!("{output_path}/text.png"))
        .unwrap();

    // 一个没有 EXIF 相机品牌（无 logo）的模板也能正常渲染出图片
    let text = Text {
        template: vec!["{拍摄日期} {光圈}".to_owned()],
        text_params: vec![TextParams {
            size: 0.03,
            ..Default::default()
        }],
    };
    let params = WatermarkParams::default();
    let fallback_img = text
        .render_text(photo.exif.as_ref().unwrap(), img_h, &params, "%Y/%m/%d")
        .unwrap()
        .expect("render_text should produce an image");
    assert!(fallback_img.width() > 0);
    assert!(fallback_img.height() > 0);
}

#[test]
fn custom_text_is_inserted_literally_alongside_exif_fields() {
    let exif = ExifInfo {
        model: Some("Camera".into()),
        ..Default::default()
    };
    assert_eq!(
        render_watermark_template("{型号} · {自定义文本}", &exif, "%Y/%m/%d", "旅途中的光"),
        "Camera · 旅途中的光"
    );
    assert_eq!(
        render_watermark_template(
            "{自定义文本} / {自定义文本}",
            &exif,
            "%Y/%m/%d",
            "{型号} {Logo}"
        ),
        "{型号} {Logo} / {型号} {Logo}"
    );
    assert_eq!(
        render_watermark_template("{型号} · {自定义文本}", &exif, "%Y/%m/%d", ""),
        "Camera ·"
    );
    assert_eq!(
        render_watermark_template("{自定义文本}", &exif, "%Y/%m/%d", ""),
        ""
    );
}

#[test]
fn templates_preserve_trailing_punctuation_even_when_fields_are_missing() {
    let exif = ExifInfo::default();
    for ending in ["-", "/", "·", "|", ",", "、"] {
        let template = format!("旅行 {ending} {{型号}}");
        assert_eq!(
            render_exif_template(&template, &exif, "%Y/%m/%d"),
            format!("旅行 {ending}")
        );
        assert_eq!(
            render_watermark_template("{自定义文本}", &exif, "%Y/%m/%d", &format!("旅行 {ending}")),
            format!("旅行 {ending}")
        );
    }
    assert_eq!(render_exif_template(",", &exif, "%Y/%m/%d"), ",");
}

#[test]
fn rich_text_renders_multiple_fonts_and_colors_in_one_line() {
    use lumen_frame_core::watermark::{TextSpan, TextStyle};
    lumen_frame_core::media::ensure_vips();
    let text = Text {
        template: vec!["WWWWiiii".into()],
        text_params: vec![TextParams {
            font: "Arial".into(),
            size: 0.08,
            color: Some([0, 0, 255]),
            spans: vec![TextSpan {
                start: 0,
                end: 4,
                style: TextStyle {
                    font: Some("Menlo".into()),
                    color: Some([255, 0, 0]),
                    ..Default::default()
                },
            }],
            ..Default::default()
        }],
    };
    let render = |text: &Text| {
        text.render_text(
            &ExifInfo::default(),
            1000,
            &WatermarkParams::default(),
            "%Y/%m/%d",
        )
        .unwrap()
        .unwrap()
    };
    let rich = render(&text);
    let pixels = lumen_frame_core::media::image_write_to_memory(&rich).unwrap();
    assert!(
        pixels
            .chunks_exact(4)
            .filter(|p| p[0] > 240 && p[1] < 10 && p[2] < 10 && p[3] > 128)
            .count()
            > 100
    );
    assert!(
        pixels
            .chunks_exact(4)
            .filter(|p| p[2] > 240 && p[1] < 10 && p[0] < 10 && p[3] > 128)
            .count()
            > 100
    );
    let mut without_font = text.clone();
    without_font.text_params[0].spans[0].style.font = None;
    assert_ne!(
        rich.width(),
        render(&without_font).width(),
        "局部字体应改变字形宽度"
    );
}

#[test]
fn gpui_font_fallback_keeps_multiline_text_and_color_glyphs() {
    let mut text = Text {
        template: vec!["Lumen 中文 ℤ 🙂".into()],
        text_params: vec![TextParams {
            font: "Lumen Frame Missing Font".into(),
            size: 0.06,
            italic: true,
            underline: true,
            color: Some([0, 0, 255]),
            ..Default::default()
        }],
    };
    let render = |text: &Text| {
        text.render_text(
            &ExifInfo::default(),
            1000,
            &WatermarkParams::default(),
            "%Y/%m/%d",
        )
        .unwrap()
        .unwrap()
    };
    let single = render(&text);
    let pixels = lumen_frame_core::media::image_write_to_memory(&single).unwrap();
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[2] > 200 && pixel[0] < 20 && pixel[3] > 100)
    );
    #[cfg(target_os = "macos")]
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[0] > 200 && pixel[1] > 100 && pixel[2] < 100 && pixel[3] > 100),
        "原生彩色 Emoji 不能退化为蓝色遮罩"
    );
    text.template[0] = format!("{0}\n{0}", text.template[0]);
    let multiline = render(&text);
    assert_eq!(multiline.width(), single.width());
    assert_eq!(multiline.height(), single.height() * 2);
}

#[test]
fn rich_dynamic_fields_keep_their_style_when_photo_values_change() {
    use lumen_frame_core::watermark::{TextSpan, TextStyle};
    lumen_frame_core::media::ensure_vips();
    let text = Text {
        template: vec!["{ISO} Blue".into()],
        text_params: vec![TextParams {
            size: 0.08,
            color: Some([0, 0, 255]),
            spans: vec![TextSpan {
                start: 0,
                end: 5,
                style: TextStyle {
                    color: Some([255, 0, 0]),
                    ..Default::default()
                },
            }],
            ..Default::default()
        }],
    };
    let mut red_counts = vec![];
    for iso in [9, 12345] {
        let exif = ExifInfo {
            isospeed_ratings: Some(iso),
            ..Default::default()
        };
        let image = text
            .render_text(&exif, 1000, &WatermarkParams::default(), "%Y/%m/%d")
            .unwrap()
            .unwrap();
        let pixels = lumen_frame_core::media::image_write_to_memory(&image).unwrap();
        red_counts.push(
            pixels
                .chunks_exact(4)
                .filter(|p| p[0] > 240 && p[2] < 10 && p[3] > 128)
                .count(),
        );
        let first_blue = pixels
            .chunks_exact(4)
            .enumerate()
            .filter(|(_, p)| p[2] > 240 && p[0] < 10 && p[3] > 128)
            .map(|(i, _)| i % image.width() as usize)
            .min()
            .unwrap();
        let last_red = pixels
            .chunks_exact(4)
            .enumerate()
            .filter(|(_, p)| p[0] > 240 && p[2] < 10 && p[3] > 128)
            .map(|(i, _)| i % image.width() as usize)
            .max()
            .unwrap();
        assert!(
            last_red < first_blue,
            "字段替换后不能让红色溢出到后面的蓝色文字"
        );
    }
    assert!(red_counts[1] > red_counts[0] * 3);
}

#[test]
fn templates_preserve_internal_spaces_around_logo_fields() {
    use lumen_frame_core::{
        render::text::render_styled_template_preview,
        watermark::{TextSpan, TextStyle},
    };
    let exif = ExifInfo {
        model: Some("Camera".into()),
        isospeed_ratings: Some(400),
        ..Default::default()
    };
    let template = "{Logo} {型号}    {自定义logo1}  {ISO} ";
    let start = template.find("{型号}").unwrap();
    let params = TextParams {
        spans: vec![TextSpan {
            start,
            end: start + "{型号}".len(),
            style: TextStyle {
                bold: Some(true),
                ..Default::default()
            },
        }],
        ..Default::default()
    };
    let (parsed, spans) = render_styled_template_preview(template, &params, &exif, "%Y/%m/%d", "");
    assert_eq!(parsed, "[品牌 Logo] Camera    [自定义logo1]  400");
    assert_eq!(&parsed[spans[0].start..spans[0].end], "Camera");
    assert_eq!(spans[0].style.bold, Some(true));
    assert_eq!(
        render_watermark_template("{型号}    {ISO} ", &exif, "%Y/%m/%d", ""),
        "Camera    400"
    );
    assert_eq!(
        render_watermark_template("{自定义文本}", &exif, "%Y/%m/%d", "A   B"),
        "A   B"
    );
}

#[test]
fn group_background_uses_alpha_shared_frame_and_automatic_text_color() {
    use lumen_frame_core::media::image_write_to_memory;
    let params = WatermarkParams {
        solid_background: true,
        background: [255; 3],
        ..Default::default()
    };
    let mut group = TextGroup::default();
    group.text.template = vec!["BACKGROUND".into()];
    group.text.text_params.truncate(1);
    group.border.set_min_width(0.4);
    group.border.set_min_height(0.12);
    group.border.set_radius(0.025);
    group.border.set_inset(0.01);
    for direction in [TextDirection::Horizontal, TextDirection::Vertical] {
        group.direction = direction;
        for (background, expected) in [
            ([0, 0, 0, 255], [255; 3]),
            ([255, 255, 255, 255], [0; 3]),
            ([0, 0, 0, 64], [0; 3]),
            ([0, 0, 0, 200], [255; 3]),
        ] {
            group.background = Some(background);
            let image = group
                .render_text(&ExifInfo::default(), 1000, &params)
                .unwrap()
                .unwrap();
            let (w, h) = (image.width() as usize, image.height() as usize);
            assert!(w >= 400 && h >= 120);
            let pixels = image_write_to_memory(&image).unwrap();
            assert_eq!(pixels[3], 0, "背景圆角之外应透明");
            assert_eq!(&pixels[(w / 2) * 4..(w / 2) * 4 + 4], &background);
            assert!(
                pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|p| p[..3] == expected && p[3] == 255),
                "自动文字应与叠加后的背景形成对比"
            );
            assert!(
                group.text.text_params[0].color.is_none(),
                "派生颜色不能写回输入"
            );
        }
    }
    group.direction = TextDirection::Horizontal;
    group.background = Some([0, 0, 0, 128]);
    group.text.text_params[0].color = Some([255, 0, 0]);
    group.border.set_enabled(true);
    group.border.set_color([0, 120, 220]);
    group.border.set_width(0.002);
    for side in [Placement::Up, Placement::Right, Placement::Bottom] {
        group.border.set_side(side, false);
    }
    let image = group
        .render_text(&ExifInfo::default(), 1000, &params)
        .unwrap()
        .unwrap();
    let (w, h) = (image.width() as usize, image.height() as usize);
    let pixels = image_write_to_memory(&image).unwrap();
    assert_eq!(
        &pixels[(h / 2 * w) * 4..(h / 2 * w) * 4 + 4],
        &[0, 120, 220, 255]
    );
    assert!(
        pixels.as_chunks::<4>().0.contains(&[255, 0, 0, 255]),
        "手动文字颜色优先"
    );
}
