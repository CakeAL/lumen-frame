//! 投影的像素契约：圆角覆盖率、连续尾部和高浓度渐变。
use lumen_frame::{
    media::load_base_image,
    render::{canvas, image as render_image},
    watermark::WatermarkParams,
};

struct FixtureFile(std::path::PathBuf);
impl Drop for FixtureFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn photo(width: u32, height: u32, alpha: u8) -> (vips::VipsImage<'static>, FixtureFile) {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let file = FixtureFile(
        std::env::temp_dir().join(format!("lumen-shadow-{}-{id}.png", std::process::id())),
    );
    image::RgbaImage::from_pixel(width, height, image::Rgba([180, 180, 180, alpha]))
        .save(&file.0)
        .unwrap();
    (load_base_image(&file.0).unwrap(), file)
}

fn shadow(width: u32, height: u32, alpha: u8, size: f64, density: f64) -> image::RgbaImage {
    let params = WatermarkParams {
        solid_background: true,
        background: [255; 3],
        border_radius: 0.015,
        shadow_size: size,
        shadow_density: density,
        ..Default::default()
    };
    let (img, _fixture) = photo(width, height, alpha);
    let img = render_image::add_round_corner(img, params.border_radius).unwrap();
    let canvas =
        canvas::new_canvas(width as i32 + 160, height as i32 + 160, &img, &params).unwrap();
    let rendered = canvas::add_shadow(canvas, &img, &params, 80, 80).unwrap();
    image::RgbaImage::from_raw(
        width + 160,
        height + 160,
        rendered.write_to_memory().unwrap(),
    )
    .unwrap()
}

#[test]
fn rounded_corners_keep_antialiasing_and_existing_transparency() {
    let (img, _fixture) = photo(100, 80, 128);
    let img = render_image::add_round_corner(img, 0.25).unwrap();
    let pixels = image::RgbaImage::from_raw(100, 80, img.write_to_memory().unwrap()).unwrap();
    assert_eq!(pixels.get_pixel(50, 40)[3], 128, "不能覆盖源图的透明度");
    assert_eq!(pixels.get_pixel(0, 0)[3], 0);
    assert!(pixels.pixels().all(|p| p[3] <= 128));
    assert!(
        pixels.pixels().filter(|p| p[3] > 0 && p[3] < 128).count() >= 20,
        "圆弧边缘必须保留部分覆盖率，不能只剩 0/255 的硬边"
    );
}

#[test]
fn transparent_pixels_do_not_cast_an_opaque_rectangular_shadow() {
    let pixels = shadow(240, 180, 0, 0.105, 2.0);
    assert!(
        pixels.pixels().all(|p| p.0 == [255; 4]),
        "透明照片不能产生黑色矩形投影"
    );
}

#[test]
fn wide_shadow_has_a_soft_tail_beyond_two_sigma() {
    // 180px 高，阴影大小 10%，sigma=6px；离左边 14px 的尾部仍应淡淡可见。
    let pixels = shadow(240, 180, 255, 0.1, 2.0);
    let near = pixels.get_pixel(79, 170)[0];
    let tail = pixels.get_pixel(66, 170)[0];
    let outside = pixels.get_pixel(50, 170)[0];
    assert!(
        tail < 254 && tail > near,
        "投影不能在约 1.8σ 处突然消失：{near}, {tail}"
    );
    assert_eq!(outside, 255, "远离照片的边框应保持背景色");
}

#[test]
fn high_density_keeps_the_edge_gradient_instead_of_clipping_to_black() {
    let normal = shadow(240, 180, 255, 0.1, 1.0);
    let strong = shadow(240, 180, 255, 0.1, 2.0);
    let normal_edge = normal.get_pixel(79, 170)[0];
    let strong_edge = strong.get_pixel(79, 170)[0];
    assert!(strong_edge < normal_edge, "提高浓度仍应加深阴影");
    assert!(
        strong_edge > 60,
        "浓度 2.0 时照片外缘也要保留灰阶：{strong_edge}"
    );
    for x in 60..79 {
        assert!(strong.get_pixel(x, 170)[0] >= strong.get_pixel(x + 1, 170)[0]);
    }
}

#[test]
fn downsampled_shadow_keeps_corners_symmetric() {
    for (photo_w, photo_h) in [(480, 600), (481, 601)] {
        let pixels = shadow(photo_w, photo_h, 255, 0.105, 2.0);
        let (width, height) = pixels.dimensions();
        for y in 40..110 {
            for x in 40..110 {
                let top_left = i32::from(pixels.get_pixel(x, y)[0]);
                for (other_x, other_y) in [(width - 1 - x, y), (x, height - 1 - y)] {
                    let difference =
                        (top_left - i32::from(pixels.get_pixel(other_x, other_y)[0])).abs();
                    assert!(
                        difference <= 3,
                        "降采样不能使左右、上下角的投影偏移：({x}, {y}), {difference}"
                    );
                }
            }
        }
    }
}
