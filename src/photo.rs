use anyhow::{Context, Result};
use libvips::{
    VipsApp,
    ops::{self, CompassDirection},
};
use nom_exif::{EntryValue, Exif, ExifDateTime, ExifTag, read_exif_async};
use std::{
    fmt::Display,
    path::{Path, PathBuf},
};

use crate::{params::WatermarkParams, process::{cal_canvas_size, cal_image_coordinates}};

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

    pub async fn generate_watermark(&self, params: &WatermarkParams) -> Result<()> {
        let _app = VipsApp::default("luman-frame").context("failed to init libvips")?;

        // 摆正原图
        let img = ops::jpegload_with_opts(
            &self.path.to_string_lossy(),
            &ops::JpegloadOptions {
                autorotate: true,
                ..Default::default()
            },
        )
        .context("failed to load image")?;

        // 计算水印照片的图片尺寸
        let (img_w, img_h) = (img.get_width(), img.get_height());
        let (canvas_w, canvas_h) = cal_canvas_size(img_w, img_h, params);
        // 计算图片坐标
        let (img_x, img_y) = cal_image_coordinates(canvas_w, canvas_h, img_w, img_h, params);

        // 生成画布
        let canvas = if params.solid_background {
            // 纯色背景
            let [r, g, b] = params.background;
            let background = ops::black(canvas_w, canvas_h)?;
            ops::linear(&background, &mut [1.0], &mut [r as f64, g as f64, b as f64])
                .context("failed to generate canvas")?
        } else {
            // 模糊背景
            // 1. 计算缩放比例，使原图完全覆盖画布 (Cover 模式)
            let scale = f64::max(
                canvas_w as f64 / img_w as f64,
                canvas_h as f64 / img_h as f64,
            );

            // 2. 等比缩放原图
            let scaled_img = ops::resize(&img, scale)?;

            // 3. 从缩放后的图片中心裁剪出画布大小（居中裁剪）
            let (scaled_w, scaled_h) = (scaled_img.get_width(), scaled_img.get_height());
            let crop_x = (scaled_w - canvas_w) / 2;
            let crop_y = (scaled_h - canvas_h) / 2;
            let background_img =
                ops::extract_area(&scaled_img, crop_x, crop_y, canvas_w, canvas_h)?;

            // 4. 对裁剪后的背景图片应用高斯模糊
            ops::gaussblur(&background_img, params.blur_sigma)
                .context("failed to generate canvas")?
        };
        // 给图片添加圆角
        if params.border_radius > 0.0 {
            crate::process::add_round_corner(&img, params.border_radius).await.context("add round corner failed")?;
        }

        // 为画布添加阴影
        let canvas = if params.shadow_size > 0.0 {
           let shadow = crate::process::generate_shadow(&img, params.shadow_size, params.shadow_opacity, params.border_radius).await.context("generate shadow failed")?;
           ops::composite2_with_opts(&canvas, &shadow, ops::BlendMode::Over, composite2_options)
        }

        Ok(())
    }
}

/// 光圈，快门速度可能是小数或者分数
#[derive(Debug, Clone)]
pub enum Rational {
    Fraction(u32, u32),
    Float(f64),
}

impl Display for Rational {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let format_value = |v: f64| {
            format!("{v:.2}")
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string()
        };
        match self {
            Rational::Fraction(n, d) => {
                let v = *n as f64 / *d as f64;
                if v >= 1.0 {
                    write!(f, "{}", format_value(v))
                } else {
                    write!(f, "{}/{}", n, d)
                }
            }
            Rational::Float(v) => write!(f, "{}", format_value(*v)),
        }
    }
}
#[derive(Debug, Default, Clone)]
pub struct ExifInfo {
    // 拍摄日期
    pub created_time: Option<ExifDateTime>,
    // 品牌
    pub make: Option<String>,
    // 机型
    pub model: Option<String>,
    // 快门速度, (1/100s)
    pub exposure_time: Option<Rational>,
    // 光圈
    pub f_number: Option<Rational>,
    // 感光度
    pub isospeed_ratings: Option<u32>,
    // 曝光补偿
    pub exposure_bias_value: Option<Rational>,
    // 实际焦距
    pub focal_length: Option<Rational>,
    // 白平衡模式
    pub white_balance_mode: Option<u16>,
    // 等效35mm焦距
    pub focal_length_in35mm_film: Option<u32>,
    // 镜头生产商
    pub lens_make: Option<String>,
    // 镜头型号
    pub lens_model: Option<String>,
}

impl ExifInfo {
    pub async fn new(path: &Path) -> Result<Self> {
        let exif = read_exif_async(path)
            .await
            .with_context(|| format!("Failed to read exif, file: {:?}", path))?;
        let get = |tag| find_value(&exif, tag);
        let to_string = |v: &EntryValue| v.as_str().map(|s| s.to_string());
        Ok(Self {
            created_time: get(ExifTag::CreateDate).and_then(|v| v.as_datetime()),
            make: get(ExifTag::Make).and_then(to_string),
            model: get(ExifTag::Model).and_then(to_string),
            exposure_time: get(ExifTag::ExposureTime).and_then(format_value),
            f_number: get(ExifTag::FNumber).and_then(format_value),
            isospeed_ratings: get(ExifTag::ISOSpeedRatings).and_then(format_iso),
            exposure_bias_value: get(ExifTag::ExposureBiasValue).and_then(format_value),
            focal_length: get(ExifTag::FocalLength).and_then(format_value),
            white_balance_mode: get(ExifTag::WhiteBalanceMode).and_then(|v| v.as_u16()),
            focal_length_in35mm_film: get(ExifTag::FocalLengthIn35mmFilm).and_then(|v| v.as_u32()),
            lens_make: get(ExifTag::LensMake).and_then(to_string),
            lens_model: get(ExifTag::LensModel).and_then(to_string),
        })
    }

    /// 渲染为底部水印文字，缺失的项自动跳过；全部缺失时返回 `None`。
    pub fn to_caption(&self) -> Option<String> {
        // [TODO] 临时
        let parts: Vec<String> = [
            self.isospeed_ratings.map(|v| v.to_string()),
            self.exposure_time.as_ref().map(|v| v.to_string()),
            self.f_number.as_ref().map(|v| v.to_string()),
            self.focal_length_in35mm_film.map(|v| v.to_string()),
        ]
        .into_iter()
        .flatten()
        .collect();

        if parts.is_empty() {
            None
        } else {
            Some(parts.join("  "))
        }
    }
}

/// 在所有 IFD 中查找指定 tag 的首个条目（拍摄参数通常位于 Exif 子 IFD 中）。
fn find_value<'a>(exif: &'a Exif, tag: ExifTag) -> Option<&'a EntryValue> {
    exif.iter()
        .find(|e| e.tag.tag() == Some(tag))
        .map(|e| e.value)
}

fn format_iso(value: &EntryValue) -> Option<u32> {
    match value {
        // 部分相机将 ISO 存为数组（如 [100]）
        EntryValue::U16Array(v) => v.first().copied().map(|n| n as u32),
        EntryValue::U32Array(v) => v.first().copied(),
        _ => value.try_as_integer().and_then(|n| u32::try_from(n).ok()),
    }
}

fn format_value(value: &EntryValue) -> Option<Rational> {
    if let Some(r) = value.as_urational() {
        let n = r.numerator();
        let d = r.denominator();
        if d == 0 {
            return None;
        }
        return Some(Rational::Fraction(n, d));
    }
    value.try_as_float().map(|s| Rational::Float(s))
}

#[cfg(test)]
mod tests {

    use crate::photo::Photo;

    #[tokio::test]
    async fn test_dump_exif() {
        let path = "./test_images/DSC_4587.jpg";
        let photo = Photo::new(&path).await.unwrap();
        dbg!(photo.exif);
    }
}
