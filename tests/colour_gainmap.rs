use std::{fs, path::Path};

use libvips::ops;
use lumen_frame::{
    features::colour_gainmap::load_black_and_white_with_colour_gainmap,
    gainmap as gain_map,
    media::{ExifInfo, load_base_image},
};

const PHOTO: &str = "/Users/cakeal/Downloads/DSC_6610.jpg";

#[test]
fn colour_gainmap_restores_colour_at_normal_display_headroom() {
    let input = Path::new(PHOTO);
    let original = load_base_image(input).expect("读取测试图片失败");
    let _has_input_gainmap = gain_map::get_gainmap(&original).is_some();

    let result =
        load_black_and_white_with_colour_gainmap(input).expect("黑白底图和彩色 gain map 处理失败");

    assert_eq!(result.get_bands(), 3, "JPEG SDR 底图应保留三个 RGB 通道");
    assert_eq!(
        result.get_interpretation().unwrap() as i32,
        ops::Interpretation::Srgb as i32,
        "底图没有标记为 sRGB"
    );
    let base_pixel = ops::getpoint(&result, 100, 100).unwrap();
    assert!(
        base_pixel
            .windows(2)
            .all(|channels| channels[0] == channels[1]),
        "不支持 gain map 的设备看到的底图应是黑白的：{base_pixel:?}"
    );

    let result_gainmap = gain_map::get_gainmap(&result).expect("处理结果丢失了 gain map");
    assert_eq!(result_gainmap.get_bands(), 3, "gain map 被降成了非彩色图");
    assert!(
        !gain_map::has_icc_profile(&result_gainmap),
        "gain map 不应继承主图的 ICC profile"
    );
    assert!(
        gain_map::get_gainmap(&result_gainmap).is_none(),
        "新生成的 gain map 不应嵌套继承输入图的 gain map"
    );
    assert_eq!(
        result_gainmap.get_interpretation().unwrap() as i32,
        ops::Interpretation::Srgb as i32,
        "gain map 应以 sRGB 交给 JPEG 编码器转换为 YUV"
    );
    assert_eq!(
        result.get_as_string("gainmap-scale-factor").unwrap(),
        "1",
        "由原图生成的 gain map 应与底图 1:1 对应"
    );
    assert!(
        result
            .get_as_string("gainmap-hdr-capacity-max")
            .unwrap()
            .parse::<f64>()
            .unwrap()
            <= 2.0,
        "显示端应在常见 HDR 余量下完整应用 gain map"
    );
    let gainmap_pixel = ops::getpoint(&result_gainmap, 100, 100).unwrap();
    assert!(
        gainmap_pixel
            .windows(2)
            .any(|channels| channels[0] != channels[1]),
        "gain map 应保留彩色通道差异：{gainmap_pixel:?}"
    );

    let output_dir = Path::new("./test_images/watermark");
    fs::create_dir_all(output_dir).unwrap();
    let file_name = format!(
        "{}_color_gainmap.jpg",
        input
            .file_stem()
            .and_then(|stem| stem.to_str())
            .expect("输入文件缺少 UTF-8 文件名")
    );
    let output = output_dir.join(file_name);
    ops::jpegsave_with_opts(
        &result,
        &output.to_string_lossy(),
        &ops::JpegsaveOptions {
            q: 95,
            keep: ops::ForeignKeep::All,
            ..Default::default()
        },
    )
    .expect("保存完整彩色 gain map 效果失败");
    gain_map::mark_jpeg_gainmap(&output).expect("写入 MPF GainMap 类型失败");
    assert!(output.is_file(), "没有生成完整效果图片");
    assert!(
        std::fs::read(&output)
            .unwrap()
            .windows(b"Item:Semantic=\"GainMap\"".len())
            .any(|item| item == b"Item:Semantic=\"GainMap\""),
        "导出 JPEG 的 MPF 未标记 GainMap 类型"
    );
    let exif = ExifInfo::read(&output).expect("彩色 Gain Map 导出后应仍能读取 EXIF");
    assert!(exif.model.is_some(), "彩色 Gain Map 导出后丢失了相机型号");
    let exported = load_base_image(&output).unwrap();
    assert!(
        gain_map::get_gainmap(&exported).is_some(),
        "重新读取导出的 JPEG 时丢失了 gain map"
    );
}
