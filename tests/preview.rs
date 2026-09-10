//! 预览位图的行为契约。
//!
//! 这一层是水印管线与 GPUI 渲染之间的接缝，也是「调整参数时预览实时变化」这句话能否
//! 成立的地方：参数进、位图出。它不需要窗口，所以可以按普通单元测试来验证。

use std::path::{Path, PathBuf};

use lumen_frame::{
    params::WatermarkParams,
    photo::ExifInfo,
    process::text::Text,
    ui::{PreviewJob, render_preview, render_thumbnail},
};

const PHOTO: &str = "./test_images/DSC_4587.jpg";

fn job(params: WatermarkParams) -> PreviewJob {
    let path = PathBuf::from(PHOTO);
    PreviewJob {
        path: path.clone(),
        exif: ExifInfo::read(Path::new(PHOTO)).ok(),
        params,
        text: Text::default(),
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
