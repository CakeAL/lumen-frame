//! 预览位图的行为契约。
//!
//! 这一层是水印管线与 GPUI 渲染之间的接缝，也是「调整参数时预览实时变化」这句话能否
//! 成立的地方：参数进、位图出。它不需要窗口，所以可以按普通单元测试来验证。

use std::path::{Path, PathBuf};

use lumen_frame::{
    Position,
    params::WatermarkParams,
    photo::ExifInfo,
    process::text::{Text, TextAlign, TextDirection, TextGroup, TextParams},
    ui::{PreviewJob, export_gainmap, render_gainmap_preview, render_preview, render_thumbnail},
};

const PHOTO: &str = "./test_images/DSC_4587.jpg";

fn job(params: WatermarkParams) -> PreviewJob {
    let path = PathBuf::from(PHOTO);
    PreviewJob {
        path: path.clone(),
        exif: ExifInfo::read(Path::new(PHOTO)).ok(),
        params,
        text_groups: vec![TextGroup::default()],
        max_edge: 1600,
    }
}

#[test]
fn exif_is_available_for_the_fixture() {
    // 文字水印是否参与合成，取决于 EXIF 能否读到；先把前提钉住，后面的断言才有意义。
    assert!(job(WatermarkParams::default()).exif.is_some());
}

#[test]
fn preview_returns_a_renderable_bitmap() {
    let image = render_preview(&job(WatermarkParams::default())).expect("预览渲染失败");

    assert_eq!(image.frame_count(), 1);
    let size = image.size(0);
    assert!(size.width.0 > 0 && size.height.0 > 0);

    // GPUI 期望 RGBA 类型的缓冲区里承载 BGRA 四通道字节。
    let bytes = image.as_bytes(0).expect("预览没有像素数据");
    assert_eq!(
        bytes.len(),
        size.width.0 as usize * size.height.0 as usize * 4
    );

    // 整幅同色说明合成没有真的发生（例如底图没读到、或者画布没被合上）。
    let first = &bytes[..4];
    assert!(
        bytes.as_chunks::<4>().0.iter().any(|pixel| pixel != first),
        "预览是纯色，合成没有生效"
    );
}

#[test]
fn preview_is_downscaled_to_stay_responsive() {
    let image = render_preview(&job(WatermarkParams::default())).unwrap();
    let size = image.size(0);

    // 底图被缩到 1600 长边；画布因为默认边框会比它更大，但必须仍在同一量级。
    assert!(
        size.width.0.max(size.height.0) < 4000,
        "预览底图没有被缩小，实时拖动会明显卡顿：{}x{}",
        size.width.0,
        size.height.0
    );
}

#[test]
fn preview_max_edge_is_part_of_the_render_request() {
    let full = render_preview(&job(WatermarkParams::default())).unwrap();
    let mut reduced_job = job(WatermarkParams::default());
    reduced_job.max_edge = 400;
    let reduced = render_preview(&reduced_job).unwrap();

    let full_size = full.size(0);
    let reduced_size = reduced.size(0);
    assert!(
        reduced_size.width.0 < full_size.width.0 && reduced_size.height.0 < full_size.height.0,
        "预览请求里的长边上限没有影响实际渲染尺寸：完整 {full_size:?}，缩小 {reduced_size:?}"
    );
}

#[test]
fn border_ratio_changes_the_preview() {
    let none = render_preview(&job(WatermarkParams {
        border_ratio: (0.0, 0.0, 0.0, 0.0),
        ..Default::default()
    }))
    .unwrap();

    let wide = render_preview(&job(WatermarkParams {
        border_ratio: (0.2, 0.2, 0.2, 0.2),
        ..Default::default()
    }))
    .unwrap();

    // 这正是界面承诺的那件事：动滑块，中间的画面跟着变。
    assert!(
        wide.size(0).width.0 > none.size(0).width.0,
        "加大边框后画布没有变宽：{} -> {}",
        none.size(0).width.0,
        wide.size(0).width.0
    );
}

#[test]
fn aspect_ratio_is_applied_to_the_preview() {
    let image = render_preview(&job(WatermarkParams {
        aspect_ratio: Some((1.0, 1.0)),
        ..Default::default()
    }))
    .unwrap();

    let size = image.size(0);
    assert_eq!(
        size.width.0, size.height.0,
        "指定 1:1 后画布不是正方形：{}x{}",
        size.width.0, size.height.0
    );
}

#[test]
fn thumbnail_stays_within_its_box() {
    let image = render_thumbnail(Path::new(PHOTO)).unwrap();
    let size = image.size(0);

    assert!(size.width.0 <= 320 && size.height.0 <= 320);
    assert!(size.width.0 > 0 && size.height.0 > 0);
}

#[test]
fn gainmap_preview_reads_the_embedded_map_and_base_image() {
    assert!(
        render_gainmap_preview(Path::new(PHOTO), true)
            .unwrap()
            .is_some(),
        "HDR 测试图片应能解析出 gain map"
    );
    assert!(
        render_gainmap_preview(Path::new(PHOTO), false)
            .unwrap()
            .is_some()
    );
}

#[test]
fn single_channel_gainmap_can_be_previewed_and_exported() {
    let source = Path::new("./test_images/ultra_hdr.jpg");
    assert!(
        render_gainmap_preview(source, true).unwrap().is_some(),
        "单通道 HDR gain map 应可转换为预览位图"
    );

    let output = std::env::temp_dir().join(format!(
        "lumen-frame-gainmap-{}-{}.png",
        std::process::id(),
        std::thread::current().name().unwrap_or("preview-test")
    ));
    let _ = std::fs::remove_file(&output);
    assert!(export_gainmap(source, &output).unwrap());
    assert!(output.is_file(), "gain map 导出文件不存在");
    std::fs::remove_file(output).unwrap();
}

#[test]
fn centred_text_lands_in_the_middle_of_the_canvas() {
    // 「居中」曾经落到 `Position` 匹配的兜底分支上，也就是画布左上角 (0, 0)，
    // 所以这里用「文字像素落在哪」来验证它真的居中。
    let text_group = TextGroup {
        position: Position::Center,
        align: TextAlign::Center,
        ..TextGroup::default()
    };

    let mut without_text = text_group.clone();
    without_text.text.template.clear();
    without_text.text.text_params.clear();

    let with_text =
        render_preview(&job_with(WatermarkParams::default(), vec![text_group])).unwrap();
    let bare = render_preview(&job_with(WatermarkParams::default(), vec![without_text])).unwrap();

    let canvas = with_text.size(0);
    let (left, top, right, bottom) =
        diff_bounds(&with_text, &bare).expect("居中位置没有渲染出文字");

    let centre_x = (left + right) as f32 / 2.0;
    let centre_y = (top + bottom) as f32 / 2.0;
    let width = canvas.width.0 as f32;
    let height = canvas.height.0 as f32;

    assert!(
        (centre_x - width / 2.0).abs() < width * 0.1,
        "文字水平方向没有居中：中心 {centre_x}，画布中心 {}",
        width / 2.0
    );
    assert!(
        (centre_y - height / 2.0).abs() < height * 0.1,
        "文字垂直方向没有居中：中心 {centre_y}，画布中心 {}",
        height / 2.0
    );
}

#[test]
fn text_groups_on_the_same_side_share_the_largest_thickness() {
    let params = WatermarkParams {
        border_ratio: (0.0, 0.0, 0.0, 0.0),
        shadow_size: 0.0,
        solid_background: true,
        ..Default::default()
    };
    let left = simple_group(Position::Up, TextAlign::Left, TextDirection::Horizontal);
    let mut right = left.clone();
    right.align = TextAlign::Right;

    let one = render_preview(&job_with(params.clone(), vec![left.clone()])).unwrap();
    let two = render_preview(&job_with(params, vec![left, right])).unwrap();

    assert_eq!(
        one.size(0),
        two.size(0),
        "同侧两组文字应共享按最大组厚度计算的边框，而不是叠加边框"
    );
}

#[test]
fn vertical_text_group_expands_canvas_by_its_rotated_height() {
    let params = WatermarkParams {
        border_ratio: (0.0, 0.0, 0.0, 0.0),
        shadow_size: 0.0,
        solid_background: true,
        ..Default::default()
    };
    let horizontal = render_preview(&job_with(
        params.clone(),
        vec![simple_group(
            Position::Up,
            TextAlign::Center,
            TextDirection::Horizontal,
        )],
    ))
    .unwrap();
    let vertical = render_preview(&job_with(
        params,
        vec![simple_group(
            Position::Up,
            TextAlign::Center,
            TextDirection::Vertical,
        )],
    ))
    .unwrap();

    assert!(
        vertical.size(0).height.0 > horizontal.size(0).height.0,
        "长文字组旋转后应按旋转后的高度扩展上方画布"
    );
}

#[test]
fn side_padding_moves_aligned_text_without_expanding_the_canvas() {
    let params = WatermarkParams {
        border_ratio: (0.0, 0.0, 0.0, 0.0),
        shadow_size: 0.0,
        solid_background: true,
        ..Default::default()
    };
    for (position, align) in [
        (Position::Up, TextAlign::Left),
        (Position::Bottom, TextAlign::Right),
        (Position::Left, TextAlign::Left),
        (Position::Right, TextAlign::Right),
    ] {
        let plain = simple_group(position, align, TextDirection::Horizontal);
        let mut padded = plain.clone();
        padded.padding = 0.08;

        let without_padding = render_preview(&job_with(params.clone(), vec![plain])).unwrap();
        let with_padding = render_preview(&job_with(params.clone(), vec![padded])).unwrap();

        assert_eq!(
            without_padding.size(0),
            with_padding.size(0),
            "{position:?} 侧对齐的文字 padding 不应撑大画布"
        );
        assert_ne!(
            without_padding.as_bytes(0),
            with_padding.as_bytes(0),
            "{position:?} 侧对齐的 padding 应改变文字的视觉位置"
        );
    }
}

fn simple_group(position: Position, align: TextAlign, direction: TextDirection) -> TextGroup {
    TextGroup {
        text: Text {
            template: vec!["LUMEN FRAME TEXT GROUP".to_owned()],
            text_params: vec![TextParams {
                size: 0.03,
                ..Default::default()
            }],
        },
        position,
        direction,
        align,
        padding: 0.0,
        time_format: "%Y/%m/%d".to_owned(),
    }
}

fn job_with(params: WatermarkParams, text_groups: Vec<TextGroup>) -> PreviewJob {
    let path = PathBuf::from(PHOTO);
    PreviewJob {
        path: path.clone(),
        exif: ExifInfo::read(Path::new(PHOTO)).ok(),
        params,
        text_groups,
        max_edge: 1600,
    }
}

/// 两张同位图里所有不同像素的外接矩形 `(left, top, right, bottom)`。
fn diff_bounds(
    left: &gpui_kit::RenderImage,
    right: &gpui_kit::RenderImage,
) -> Option<(u32, u32, u32, u32)> {
    let size = left.size(0);
    assert_eq!(size, right.size(0), "两张图尺寸必须一致才能逐像素比较");
    let (width, height) = (size.width.0 as u32, size.height.0 as u32);

    let a = left.as_bytes(0).expect("缺少像素数据");
    let b = right.as_bytes(0).expect("缺少像素数据");

    let (mut min_x, mut min_y) = (u32::MAX, u32::MAX);
    let (mut max_x, mut max_y) = (0u32, 0u32);
    let mut found = false;

    for y in 0..height {
        for x in 0..width {
            let offset = ((y * width + x) * 4) as usize;
            if a[offset..offset + 4] != b[offset..offset + 4] {
                found = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }

    found.then_some((min_x, min_y, max_x, max_y))
}
