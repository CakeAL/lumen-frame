pub(crate) mod layers;

use anyhow::{Context, Result, anyhow};
use std::path::{Path, PathBuf};
use vips::VipsBlendMode;
use vips_sys::VipsForeignKeep;

use crate::{
    gainmap as gain_map,
    media::{
        ExifInfo, ensure_vips, load_base_image,
        vips::{VipsImage, from_owned_ptr, image_metadata_string, image_op},
    },
    render::{canvas, image, text::TextGroupRegion},
    watermark::{TextGroup, WatermarkParams},
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
        Self::compose_watermark_impl(img, exif, params, text_groups, None)
    }

    /// 预览复用同一合成管线，并取出每个已显示文字组的真实范围。
    #[cfg(test)]
    pub(crate) fn compose_watermark_with_regions(
        img: VipsImage,
        exif: Option<&ExifInfo>,
        params: &WatermarkParams,
        text_groups: &[TextGroup],
    ) -> Result<(VipsImage, Vec<TextGroupRegion>)> {
        let mut regions = Vec::new();
        let image =
            Self::compose_watermark_impl(img, exif, params, text_groups, Some(&mut regions))?;
        Ok((image, regions))
    }

    fn compose_watermark_impl(
        img: VipsImage,
        exif: Option<&ExifInfo>,
        params: &WatermarkParams,
        text_groups: &[TextGroup],
        mut regions: Option<&mut Vec<TextGroupRegion>>,
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

        let mut renderer = VipsLayerRenderer {
            img: &img,
            exif,
            params,
            rounded: None,
        };
        let scene =
            Self::prepare_watermark_layers(&mut renderer, (img_w, img_h), params, text_groups)?;
        let (canvas_w, canvas_h) = scene.size;
        let (img_x, img_y) = scene.photo_origin;
        let mut layers = scene.layers.into_iter();
        let mut canvas = layers.next().expect("背景图层").image;
        for layer in layers {
            if let (Some(regions), Some(group_ix)) = (regions.as_deref_mut(), layer.group_ix)
                && let Some(region) = TextGroupRegion::from_layer(
                    &layer.image,
                    group_ix,
                    (layer.x, layer.y),
                    scene.size,
                    params.rotation,
                )?
            {
                regions.push(region);
            }
            canvas = image_op(|out| unsafe {
                vips_sys::vips_composite2(
                    canvas.as_ptr(),
                    layer.image.as_ptr(),
                    out,
                    VipsBlendMode::VIPS_BLEND_MODE_OVER,
                    c"x".as_ptr(),
                    layer.x,
                    c"y".as_ptr(),
                    layer.y,
                    std::ptr::null::<i8>(),
                )
            })
            .context("合成水印图层失败")?;
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

    /// 完整合成与分层预览共用这条排版管线，避免缓存另做一套照片/文字位置规则。
    pub(crate) fn prepare_watermark_layers<R: layers::LayerRenderer>(
        renderer: &mut R,
        image_size: (i32, i32),
        params: &WatermarkParams,
        text_groups: &[TextGroup],
    ) -> Result<layers::WatermarkLayers<R::Image>> {
        layers::prepare(renderer, image_size, params, text_groups)
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

            // thumbnail_image 会以小块请求上游像素，直接遍历合成管线会让模糊、阴影等
            // 操作非常慢。先一次性求值成片，缩略图与 JPEG 编码共用这份 RGB 像素。
            flattened =
                unsafe { from_owned_ptr(vips_sys::vips_image_copy_memory(flattened.as_ptr())) }
                    .context("准备导出像素失败")?;
            crate::media::jpeg::update_thumbnail(&mut flattened)?;

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

struct VipsLayerRenderer<'a> {
    img: &'a VipsImage,
    exif: Option<&'a ExifInfo>,
    params: &'a WatermarkParams,
    rounded: Option<VipsImage>,
}

impl layers::LayerRenderer for VipsLayerRenderer<'_> {
    type Image = VipsImage;

    fn dimensions(image: &VipsImage) -> (i32, i32) {
        (image.width() as i32, image.height() as i32)
    }

    fn content_bounds(image: &VipsImage) -> Result<(i32, i32, i32, i32)> {
        let (w, h) = Self::dimensions(image);
        Ok(
            TextGroupRegion::from_layer(image, 0, (0, 0), (w, h), crate::rotation::Rotation::None)?
                .map(|region| (region.x, region.y, region.width, region.height))
                .unwrap_or((0, 0, w, h)),
        )
    }

    fn text(&mut self, group: &TextGroup) -> Result<Option<VipsImage>> {
        Ok(group.render_for_photo(self.exif, self.img.height() as i32, self.params)?)
    }

    fn background(&mut self, (w, h): (i32, i32)) -> Result<VipsImage> {
        Ok(canvas::new_canvas(w, h, self.img, self.params)?)
    }

    fn photo(&mut self) -> Result<VipsImage> {
        if self.rounded.is_none() {
            let img = image_op(|out| unsafe {
                vips_sys::vips_copy(self.img.as_ptr(), out, std::ptr::null::<i8>())
            })?;
            self.rounded = Some(if self.params.border_radius > 0.0 {
                image::add_round_corner(img, self.params.border_radius)?
            } else {
                img
            });
        }
        // 独立 GObject 引用，不能依赖 vips crate 的浅拷贝 Clone。
        Ok(image_op(|out| unsafe {
            vips_sys::vips_copy(
                self.rounded.as_ref().unwrap().as_ptr(),
                out,
                std::ptr::null::<i8>(),
            )
        })?)
    }

    fn shadow(&mut self) -> Result<Option<layers::PositionedLayer<VipsImage>>> {
        let img = self.photo()?;
        Ok(
            canvas::shadow_layer(&img, self.params)?.map(|(image, x, y)| layers::PositionedLayer {
                image,
                x,
                y,
                group_ix: None,
            }),
        )
    }
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
