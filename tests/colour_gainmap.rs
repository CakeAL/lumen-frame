use std::{fs, path::Path};

use lumen_frame::{
    features::colour_gainmap::load_black_and_white_with_colour_gainmap,
    gainmap as gain_map,
    media::{ExifInfo, image_metadata_string, load_base_image},
    rotation::Rotation,
};
use vips::VipsInterpretation;
use vips_sys::VipsForeignKeep;

const PHOTO: &str = "./test_images/ultra_hdr.jpg";

#[test]
fn colour_gainmap_restores_colour_at_normal_display_headroom() {
    let input = Path::new(PHOTO);
    let original = load_base_image(input).expect("读取测试图片失败");
    let _has_input_gainmap = gain_map::get_gainmap(&original).is_some();

    let result = load_black_and_white_with_colour_gainmap(input, Rotation::None)
        .expect("黑白底图和彩色 gain map 处理失败");

    assert_eq!(result.bands(), 3, "JPEG SDR 底图应保留三个 RGB 通道");
    assert_eq!(
        unsafe { vips_sys::vips_image_get_interpretation(result.as_ptr()) } as i32,
        VipsInterpretation::VIPS_INTERPRETATION_sRGB as i32,
        "底图没有标记为 sRGB"
    );
    let base_pixel = getpoint(&result, 100, 100);
    assert!(
        base_pixel
            .windows(2)
            .all(|channels| channels[0] == channels[1]),
        "不支持 gain map 的设备看到的底图应是黑白的：{base_pixel:?}"
    );

    let result_gainmap = gain_map::get_gainmap(&result).expect("处理结果丢失了 gain map");
    assert_eq!(result_gainmap.bands(), 3, "gain map 被降成了非彩色图");
    assert!(
        !gain_map::has_icc_profile(&result_gainmap),
        "gain map 不应继承主图的 ICC profile"
    );
    assert!(
        gain_map::get_gainmap(&result_gainmap).is_none(),
        "新生成的 gain map 不应嵌套继承输入图的 gain map"
    );
    assert_eq!(
        unsafe { vips_sys::vips_image_get_interpretation(result_gainmap.as_ptr()) } as i32,
        VipsInterpretation::VIPS_INTERPRETATION_sRGB as i32,
        "gain map 应以 sRGB 交给 JPEG 编码器转换为 YUV"
    );
    assert_eq!(
        image_metadata_string(&result, "gainmap-scale-factor").unwrap(),
        "1",
        "由原图生成的 gain map 应与底图 1:1 对应"
    );
    assert!(
        image_metadata_string(&result, "gainmap-hdr-capacity-max")
            .unwrap()
            .parse::<f64>()
            .unwrap()
            <= 2.0,
        "显示端应在常见 HDR 余量下完整应用 gain map"
    );
    let gainmap_pixel = getpoint(&result_gainmap, 100, 100);
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
    let output_c = std::ffi::CString::new(output.to_string_lossy().as_bytes()).unwrap();
    vips::code_to_result(unsafe {
        vips_sys::vips_jpegsave(
            result.as_ptr(),
            output_c.as_ptr(),
            c"Q".as_ptr(),
            95_i32,
            c"keep".as_ptr(),
            VipsForeignKeep::VIPS_FOREIGN_KEEP_ALL,
            std::ptr::null::<i8>(),
        )
    })
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

#[test]
fn colour_gainmap_rotates_base_and_gainmap_together() {
    let input = Path::new(PHOTO);
    let original =
        load_black_and_white_with_colour_gainmap(input, Rotation::None).expect("生成原方向失败");
    let rotated = load_black_and_white_with_colour_gainmap(input, Rotation::Clockwise90)
        .expect("生成旋转方向失败");
    let rotated_gainmap = gain_map::get_gainmap(&rotated).expect("旋转结果丢失 gain map");

    assert_eq!(rotated.width(), original.height());
    assert_eq!(rotated.height(), original.width());
    assert_eq!(rotated_gainmap.width(), rotated.width());
    assert_eq!(rotated_gainmap.height(), rotated.height());

    rotated.write_to_memory().expect("旋转底图求值失败");
    rotated_gainmap
        .write_to_memory()
        .expect("旋转 gain map 求值失败");
}

fn getpoint(image: &vips::VipsImage<'_>, x: i32, y: i32) -> Vec<f64> {
    let mut values = std::ptr::null_mut();
    let mut count = 0;
    vips::code_to_result(unsafe {
        vips_sys::vips_getpoint(
            image.as_ptr(),
            &mut values,
            &mut count,
            x,
            y,
            std::ptr::null::<i8>(),
        )
    })
    .unwrap();
    assert!(!values.is_null());
    let result = unsafe { std::slice::from_raw_parts(values, count as usize).to_vec() };
    unsafe { vips_sys::g_free(values.cast()) };
    result
}
