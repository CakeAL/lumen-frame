use anyhow::{Context, Result, anyhow};
use std::path::{Path, PathBuf};
use vips::VipsBlendMode;
use vips_sys::VipsForeignKeep;

use crate::{
    gainmap as gain_map,
    media::{
        ExifInfo, ensure_vips, load_base_image,
        vips::{VipsImage, image_metadata_string, image_op},
    },
    render::{canvas, image},
    watermark::{Placement, TextAlign, TextGroup, WatermarkParams},
};

#[derive(Debug, Clone)]
pub struct Photo {
    pub path: PathBuf,
    pub is_motion_photo: bool,
    pub is_ultra_hdr_photo: bool,
    pub exif: Option<ExifInfo>,
}

impl Photo {
    pub async fn new(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            path: path.as_ref().to_path_buf(),
            is_motion_photo: false,
            is_ultra_hdr_photo: false,
            exif: Some(ExifInfo::new(path.as_ref()).await?),
        })
    }

    /// 同步打开一张照片。
    ///
    /// GUI 的后台线程没有 tokio 运行时，[`Photo::new`] 依赖的 `tokio::fs` 在那儿会
    /// panic，所以界面走这条路径。EXIF 读不出来不算致命：照片照常处理，只是不渲染
    /// 文字水印。
    pub fn open_blocking(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        Ok(Self {
            path: path.to_path_buf(),
            is_motion_photo: false,
            is_ultra_hdr_photo: false,
            exif: ExifInfo::read(path).ok(),
        })
    }

    pub fn generate_watermark(
        &self,
        params: &WatermarkParams,
        text_groups: &[TextGroup],
    ) -> Result<VipsImage> {
        let img = load_base_image(&self.path)?;
        Self::compose_watermark(img, self.exif.as_ref(), params, text_groups)
    }

    /// 在给定的底图上合成水印。
    ///
    /// 与 [`Self::generate_watermark`] 的区别只有底图来源：导出时是原图，实时预览时
    /// 是缩略后的底图。把这条管线抽出来，预览才能复用导出完全相同的合成逻辑。
    pub fn compose_watermark(
        img: VipsImage,
        exif: Option<&ExifInfo>,
        params: &WatermarkParams,
        text_groups: &[TextGroup],
    ) -> Result<VipsImage> {
        ensure_vips();

        // 如果原图是 Ultra HDR，先取出 gain map（后面要重新生成只覆盖照片区域的版本）。
        let original_gainmap = gain_map::get_gainmap(&img);
        // gain map 的分辨率比例（1 表示与 base 同尺寸，2 表示一半尺寸……）
        let gainmap_scale = original_gainmap.as_ref().map(|_| {
            image_metadata_string(&img, "gainmap-scale-factor")
                .ok()
                .and_then(|s| s.trim().parse::<f64>().ok())
                .unwrap_or(2.0)
        });

        // 计算水印照片的图片尺寸
        let (img_w, img_h) = (img.width() as i32, img.height() as i32);

        // 先渲染每个文字组，旋转后的真实尺寸才能准确决定四周需要扩出多少画布。
        let mut rendered_text_groups = Vec::new();
        if let Some(exif) = exif {
            for group in text_groups {
                if let Some(image) = group.render_text(exif, img_h, params)? {
                    rendered_text_groups.push((group.position, group.align, image));
                }
            }
        }

        // 计算画布边框尺寸
        let mut margin = canvas::Margin::cal_margin(img_w, img_h, params);
        let mut text_thickness = [0; 4];
        for (position, _, image) in &rendered_text_groups {
            let thickness = match position {
                Placement::Up | Placement::Bottom => image.height() as i32,
                Placement::Left | Placement::Right => image.width() as i32,
                Placement::Center => 0,
            };
            let slot = match position {
                Placement::Up => Some(0),
                Placement::Right => Some(1),
                Placement::Bottom => Some(2),
                Placement::Left => Some(3),
                Placement::Center => None,
            };
            if let Some(slot) = slot {
                text_thickness[slot] = text_thickness[slot].max(thickness);
            }
        }
        margin.include_text_thickness(Placement::Up, text_thickness[0]);
        margin.include_text_thickness(Placement::Right, text_thickness[1]);
        margin.include_text_thickness(Placement::Bottom, text_thickness[2]);
        margin.include_text_thickness(Placement::Left, text_thickness[3]);
        // 画布尺寸
        let (canvas_w, canvas_h) = canvas::cal_size(&margin, img_w, img_h, params);
        // 计算图片坐标
        let (img_x, img_y) =
            image::cal_coordinates(&margin, canvas_w, canvas_h, img_w, img_h, params);

        // 生成画布
        let canvas = canvas::new_canvas(canvas_w, canvas_h, &img, params)
            .context("failed to generate canvas")?;

        // 给图片添加圆角
        let img = if params.border_radius > 0.0 {
            image::add_round_corner(img, params.border_radius).context("add round corner failed")?
        } else {
            img
        };

        // 为画布添加阴影
        let canvas = if params.shadow_size > 0.0 {
            canvas::add_shadow(canvas, &img, params, img_x, img_y)
                .context("generate shadow failed")?
        } else {
            canvas
        };

        // 合成照片
        let canvas = image_op(|out| unsafe {
            vips_sys::vips_composite2(
                canvas.as_ptr(),
                img.as_ptr(),
                out,
                VipsBlendMode::VIPS_BLEND_MODE_OVER,
                c"x".as_ptr(),
                img_x,
                c"y".as_ptr(),
                img_y,
                std::ptr::null::<i8>(),
            )
        })
        .context("composite image err")?;

        // 各文字组独立按组级位置和对齐方式合成。行级 align 已在组内排版时生效。
        let mut canvas = canvas;
        for (position, align, text_layer) in rendered_text_groups {
            let (text_w, text_h) = (text_layer.width() as i32, text_layer.height() as i32);
            let aligned_x = || match align {
                TextAlign::Left => img_x,
                TextAlign::Center => img_x + (img_w - text_w) / 2,
                TextAlign::Right => img_x + img_w - text_w,
            };
            let aligned_y = || match align {
                TextAlign::Left => img_y,
                TextAlign::Center => img_y + (img_h - text_h) / 2,
                TextAlign::Right => img_y + img_h - text_h,
            };
            let (text_x, text_y) = match position {
                Placement::Up => (aligned_x(), (img_y - text_h) / 2),
                Placement::Bottom => (
                    aligned_x(),
                    img_y + img_h + (canvas_h - img_y - img_h - text_h) / 2,
                ),
                Placement::Left => ((img_x - text_w) / 2, aligned_y()),
                Placement::Right => (
                    img_x + img_w + (canvas_w - img_x - img_w - text_w) / 2,
                    aligned_y(),
                ),
                Placement::Center => (aligned_x(), img_y + (img_h - text_h) / 2),
            };
            canvas = image_op(|out| unsafe {
                vips_sys::vips_composite2(
                    canvas.as_ptr(),
                    text_layer.as_ptr(),
                    out,
                    VipsBlendMode::VIPS_BLEND_MODE_OVER,
                    c"x".as_ptr(),
                    text_x,
                    c"y".as_ptr(),
                    text_y,
                    std::ptr::null::<i8>(),
                )
            })
            .context("composite text layer err")?;
        }

        // 原图带 Ultra HDR gain map 时，重写一个只覆盖中间照片区域、四周为
        // boost=1（0）的新 gain map，避免水印边框/背景被额外提亮。
        if let Some(original_gainmap) = original_gainmap.as_ref() {
            // 按原图的 gainmap-scale-factor 生成，不提前放大/缩小 gain map。
            let new_gainmap = gain_map::make_watermark_gainmap(
                original_gainmap,
                canvas_w,
                canvas_h,
                img_x,
                img_y,
                img_w,
                img_h,
                gainmap_scale.unwrap_or(2.0),
                params.border_radius,
            )?;
            gain_map::set_gainmap(&mut canvas, &new_gainmap);
        }

        // 旋转属于最终输出变换：照片、边框、文字和重建后的 gain map 一起旋转，不能在
        // 排版前先旋转输入照片，否则四周边框与文字位置会被重新计算。
        crate::rotation::apply_to_output(&canvas, params.rotation)
    }

    pub fn save_image(&self, params: &WatermarkParams, watermark: &VipsImage) -> Result<()> {
        ensure_vips();

        let stem = self
            .path
            .file_stem()
            .and_then(|x| x.to_str())
            .unwrap_or("_");
        // [TODO]多格式支持
        let extension = "jpg";
        if let Some(output_folder) = &params.output_folder {
            if !output_folder.exists() {
                std::fs::create_dir_all(output_folder)?
            }
            let output_path = output_folder.join(format!("{stem}_watermark.{extension}"));

            // 合成结果带 alpha（4 band），写出 JPEG 前先压平为 3 band RGB。
            let [r, g, b] = params.background;
            let background = [r as f64, g as f64, b as f64];
            let array = unsafe { vips_sys::vips_array_double_new(background.as_ptr(), 3) };
            if array.is_null() {
                return Err(anyhow!("无法创建 JPEG 背景色"));
            }
            let flatten_result = image_op(|out| unsafe {
                vips_sys::vips_flatten(
                    watermark.as_ptr(),
                    out,
                    c"background".as_ptr(),
                    array,
                    std::ptr::null::<i8>(),
                )
            });
            unsafe { vips_sys::vips_area_unref(array.cast()) };
            let mut flattened = flatten_result.context("flatten image failed")?;

            // libuhdr 只接受边长不超过 8192 的 Ultra HDR 图像。普通 JPEG 不走这条
            // 编码器，绝不能因为同一个上限被悄悄缩小；只有确实带 gain map 的输出才缩放。
            let max_dim = flattened.width().max(flattened.height()) as i32;
            if let Some(scale) =
                ultra_hdr_resize_scale(max_dim, gain_map::get_gainmap(&flattened).is_some())
            {
                flattened = flattened
                    .resize(scale, None, None)
                    .context("resize for UHDR limit failed")?;
            }

            // 保留源图的 ICC（如 Display P3），让照片保持原色域。
            // profile: None 表示不要用 libvips 默认的 sRGB profile 覆盖它。
            let output = std::ffi::CString::new(output_path.to_string_lossy().as_bytes())?;
            vips::code_to_result(unsafe {
                vips_sys::vips_jpegsave(
                    flattened.as_ptr(),
                    output.as_ptr(),
                    c"Q".as_ptr(),
                    params.quality,
                    c"keep".as_ptr(),
                    VipsForeignKeep::VIPS_FOREIGN_KEEP_ALL,
                    std::ptr::null::<i8>(),
                )
            })
            .context("save jpg failed")?;
            if gain_map::get_gainmap(&flattened).is_some() {
                gain_map::mark_jpeg_gainmap(&output_path)?;
            }
            Ok(())
        } else {
            Err(anyhow!("no output folder specified"))
        }
    }
}

/// libuhdr 的输入上限只适用于带 gain map 的 Ultra HDR 编码路径。
fn ultra_hdr_resize_scale(max_dim: i32, has_gainmap: bool) -> Option<f64> {
    const ULTRA_HDR_MAX_EDGE: i32 = 8192;
    (has_gainmap && max_dim > ULTRA_HDR_MAX_EDGE)
        .then_some(ULTRA_HDR_MAX_EDGE as f64 / max_dim as f64)
}

#[cfg(test)]
mod export_tests {
    use super::ultra_hdr_resize_scale;

    #[test]
    fn only_ultra_hdr_exports_are_limited_to_8192() {
        assert_eq!(ultra_hdr_resize_scale(9_000, false), None);
        assert_eq!(ultra_hdr_resize_scale(8_192, true), None);
        assert_eq!(ultra_hdr_resize_scale(16_384, true), Some(0.5));
    }
}
