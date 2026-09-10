use anyhow::{Context, Result, anyhow};
use libvips::{VipsApp, VipsImage, ops};
use nom_exif::{EntryValue, Exif, ExifDateTime, ExifTag, read_exif_async};
use num_integer::Integer;
use std::{
    fmt::Display,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use crate::{Position, params::WatermarkParams, process::*};

/// 进程级单例：保证 libvips 在整个进程生命周期内保持初始化。
///
/// `VipsApp` 的 Drop 会调用 `vips_shutdown`，它会释放掉所有仍存活的 `VipsImage`
/// （包括 `generate_watermark` 返回的那张）。因此不能每次调用都 init/shutdown，
/// 否则返回的图片和后续的 `save_image` 都会变成 use-after-free。
fn vips() -> &'static VipsApp {
    static VIPS: OnceLock<VipsApp> = OnceLock::new();
    VIPS.get_or_init(|| VipsApp::default("lumen-frame").expect("failed to init libvips"))
}

/// 保证 libvips 已初始化，并返回进程级单例。
///
/// **任何 vips 调用之前都必须先走这里。** libvips 的操作类哈希是首次使用时惰性构建的
/// （`vips_operation_new` → GLib 的 `g_once`）；如果两个线程同时第一次触碰 vips，或者在
/// `vips_init` 之前就调用，就会崩在 `vips_class_map_all` 里 —— 这是真实发生过的段错误。
///
/// 队列缩略图、预览合成、导出各跑在后台线程池的不同线程上，所以这个入口不能只存在于
/// 其中某一条路径里。
pub fn ensure_vips() -> &'static VipsApp {
    vips()
}

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

    /// 加载原图并按 EXIF orientation 摆正。
    ///
    /// 用自动检测加载器读取，这样 Ultra HDR (ISO 21496-1) 的 gain map 会被保留在
    /// `VipsImage` 上，后续处理（缩放、合成等）会带着它一起走。
    pub fn load_base_image(path: &Path) -> Result<VipsImage> {
        vips();

        let img =
            VipsImage::new_from_file(&path.to_string_lossy()).context("failed to load image")?;
        ops::autorot(&img).context("failed to autorotate image")
    }

    pub fn generate_watermark(
        &self,
        params: &WatermarkParams,
        text: &text::Text,
    ) -> Result<VipsImage> {
        let img = Self::load_base_image(&self.path)?;
        Self::compose_watermark(img, self.exif.as_ref(), params, text)
    }

    /// 在给定的底图上合成水印。
    ///
    /// 与 [`Self::generate_watermark`] 的区别只有底图来源：导出时是原图，实时预览时
    /// 是缩略后的底图。把这条管线抽出来，预览才能复用导出完全相同的合成逻辑。
    pub fn compose_watermark(
        img: VipsImage,
        exif: Option<&ExifInfo>,
        params: &WatermarkParams,
        text: &text::Text,
    ) -> Result<VipsImage> {
        vips();

        // 如果原图是 Ultra HDR，先取出 gain map（后面要重新生成只覆盖照片区域的版本）
        let original_gainmap = gain_map::get_gainmap(&img);
        // gain map 的分辨率比例（1 表示与 base 同尺寸，2 表示一半尺寸……）
        let gainmap_scale = original_gainmap.as_ref().map(|_| {
            img.get_as_string("gainmap-scale-factor")
                .ok()
                .and_then(|s| s.trim().parse::<f64>().ok())
                .unwrap_or(2.0)
        });

        // 计算水印照片的图片尺寸
        let (img_w, img_h) = (img.get_width(), img.get_height());

        // 计算文字占用尺寸
        let (text_position, text_height) = if exif.is_some() {
            text.cal_height(img_h)
        } else {
            (Position::Bottom, 0)
        };
        // 计算画布边框尺寸
        let margin = canvas::Margin::cal_margin(img_w, img_h, text_height, text_position, params);
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
        let canvas = ops::composite2_with_opts(
            &canvas,
            &img,
            ops::BlendMode::Over,
            &ops::Composite2Options {
                x: img_x,
                y: img_y,
                ..Default::default()
            },
        )
        .context("composite image err")?;

        // 渲染字体图片
        let text_layer = if let Some(exif) = exif {
            text.render_text(exif, img_h, params)?
        } else {
            None
        };
        let mut canvas = if let Some(mut text_layer) = text_layer {
            let (text_w, text_h) = (text_layer.get_width(), text_layer.get_height());
            let (text_x, text_y) = match text.position {
                Position::Up => (img_x + img_w / 2 - text_w / 2, (margin.top - text_h) / 2),
                Position::Bottom => (
                    img_x + img_w / 2 - text_w / 2,
                    canvas_h - (text_h + margin.bottom) / 2,
                ),
                Position::Left => {
                    text_layer = ops::rot(&text_layer, libvips::ops::Angle::D90)?;
                    let (text_w, text_h) = (text_layer.get_width(), text_layer.get_height());
                    (
                        (margin.left - text_w) / 2,
                        margin.top + img_h / 2 - text_h / 2,
                    )
                }
                Position::Right => {
                    text_layer = ops::rot(&text_layer, libvips::ops::Angle::D90)?;
                    let (text_w, text_h) = (text_layer.get_width(), text_layer.get_height());
                    (
                        canvas_w - (text_w + margin.right) / 2,
                        img_y + img_h / 2 - text_h / 2,
                    )
                }
                // 居中：压在照片正中。此时 `Margin::cal_margin` 不为文字留边距，
                // 中心点就是照片中心。
                Position::Center => (
                    img_x + img_w / 2 - text_w / 2,
                    img_y + img_h / 2 - text_h / 2,
                ),
            };
            ops::composite2_with_opts(
                &canvas,
                &text_layer,
                ops::BlendMode::Over,
                &ops::Composite2Options {
                    x: text_x,
                    y: text_y,
                    ..Default::default()
                },
            )
            .context("composite text layer err")?
        } else {
            canvas
        };

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

        Ok(canvas)
    }

    pub fn save_image(&self, params: &WatermarkParams, watermark: &VipsImage) -> Result<()> {
        vips();

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
            let mut flattened = ops::flatten_with_opts(
                watermark,
                &ops::FlattenOptions {
                    background: vec![r as f64, g as f64, b as f64],
                    ..Default::default()
                },
            )
            .context("flatten image failed")?;

            // libuhdr 上限 8192x8192。若水印超限，直接整图等比例缩到 8192，
            // gain map 会跟随一起缩放，之后仍以 Ultra HDR 保存，不缩放其它内容。
            let max_dim = flattened.get_width().max(flattened.get_height());
            if max_dim > 8192 {
                let scale = 8192.0 / max_dim as f64;
                flattened =
                    ops::resize(&flattened, scale).context("resize for UHDR limit failed")?;
            }

            // 保留源图的 ICC（如 Display P3），让照片保持原色域。
            // profile: None 表示不要用 libvips 默认的 sRGB profile 覆盖它。
            ops::jpegsave_with_opts(
                &flattened,
                &output_path.to_string_lossy(),
                &ops::JpegsaveOptions {
                    q: params.quality,
                    // 保留 Ultra HDR gain map 等元数据
                    keep: ops::ForeignKeep::All,
                    profile: None,
                    ..Default::default()
                },
            )
            .context("save jpg failed")
        } else {
            Err(anyhow!("no output folder specified"))
        }
    }
}

/// 光圈，快门速度可能是小数或者分数
#[derive(Debug, Clone)]
pub enum Rational {
    Fraction(i32, i32),
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
                let gcd = n.gcd(d);
                let n = n / gcd;
                let d = d / gcd;
                if d == 1 {
                    write!(f, "{}", n)
                } else {
                    let v = n as f64 / d as f64;
                    if v >= 1.0 {
                        write!(f, "{}", format_value(v))
                    } else {
                        write!(f, "{}/{}", n, d)
                    }
                }
            }
            Rational::Float(v) => {
                if v < &1.0 {
                    let d = (1.0 / v).round() as u32;
                    write!(f, "1/{}", d)
                } else {
                    write!(f, "{}", format_value(*v))
                }
            }
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
    pub focal_length_in35mm_film: Option<u16>,
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
        Ok(Self::from_exif(&exif))
    }

    /// 阻塞读取 EXIF。供没有 tokio 运行时的后台线程使用。
    pub fn read(path: &Path) -> Result<Self> {
        let exif = nom_exif::read_exif(path)
            .with_context(|| format!("Failed to read exif, file: {:?}", path))?;
        Ok(Self::from_exif(&exif))
    }

    fn from_exif(exif: &Exif) -> Self {
        let get = |tag| find_value(exif, tag);
        let to_string = |v: &EntryValue| v.as_str().map(|s| s.to_string());
        Self {
            created_time: get(ExifTag::CreateDate).and_then(|v| v.as_datetime()),
            make: get(ExifTag::Make).and_then(to_string),
            model: get(ExifTag::Model).and_then(to_string),
            exposure_time: get(ExifTag::ExposureTime).and_then(format_value),
            f_number: get(ExifTag::FNumber).and_then(format_value),
            isospeed_ratings: get(ExifTag::ISOSpeedRatings).and_then(format_iso),
            exposure_bias_value: get(ExifTag::ExposureBiasValue).and_then(format_value),
            focal_length: get(ExifTag::FocalLength).and_then(format_value),
            white_balance_mode: get(ExifTag::WhiteBalanceMode).and_then(|v| v.as_u16()),
            focal_length_in35mm_film: get(ExifTag::FocalLengthIn35mmFilm).and_then(|v| v.as_u16()),
            lens_make: get(ExifTag::LensMake).and_then(to_string),
            lens_model: get(ExifTag::LensModel).and_then(to_string),
        }
    }
}

/// 在所有 IFD 中查找指定 tag 的首个条目（拍摄参数通常位于 Exif 子 IFD 中）。
fn find_value(exif: &Exif, tag: ExifTag) -> Option<&EntryValue> {
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
    if let Some(r) = value.as_irational() {
        let n = r.numerator();
        let d = r.denominator();
        if d == 0 {
            return None;
        }
        return Some(Rational::Fraction(n, d));
    }
    value.try_as_float().map(Rational::Float)
}
