//! 当前照片的 EXIF 查看与覆盖编辑 Sheet。
//!
//! 输入框只保存尚未提交的草稿；照片队列中的 [`ExifInfo`] 仍是预览与导出的唯一真值。
//! 用户点击“应用”后一次性解析并写回，原始照片文件不会被修改。

use chrono::NaiveDateTime;
use gpui_kit::component::{
    WindowExt as _,
    button::{Button, ButtonVariants as _},
    group_box::GroupBox,
    h_flex,
    input::{Input, InputState},
    notification::Notification,
    v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{App, Context, Entity, Render, SharedString, Window, px};
use nom_exif::{ExifDateTime, GPSInfo};

use crate::{
    media::{ExifInfo, Rational},
    workspace::PhotoId,
};

use super::super::AppView;
use super::field::{field, hint, warning};

const DATE_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

struct ExifInputs {
    created_time: Entity<InputState>,
    make: Entity<InputState>,
    model: Entity<InputState>,
    lens_make: Entity<InputState>,
    lens_model: Entity<InputState>,
    exposure_time: Entity<InputState>,
    f_number: Entity<InputState>,
    iso: Entity<InputState>,
    exposure_bias: Entity<InputState>,
    focal_length: Entity<InputState>,
    focal_length_35mm: Entity<InputState>,
    white_balance: Entity<InputState>,
    gps: Entity<InputState>,
}

impl ExifInputs {
    fn new(exif: &ExifInfo, window: &mut Window, cx: &mut Context<ExifEditor>) -> Self {
        Self {
            created_time: input(
                format_created_time(exif.created_time),
                "例如 2026-09-24 18:30:00",
                window,
                cx,
            ),
            make: input(
                exif.make.clone().unwrap_or_default(),
                "例如 Nikon",
                window,
                cx,
            ),
            model: input(
                exif.model.clone().unwrap_or_default(),
                "例如 Z50II",
                window,
                cx,
            ),
            lens_make: input(
                exif.lens_make.clone().unwrap_or_default(),
                "例如 Nikon",
                window,
                cx,
            ),
            lens_model: input(
                exif.lens_model.clone().unwrap_or_default(),
                "例如 NIKKOR Z 24-70mm f/4 S",
                window,
                cx,
            ),
            exposure_time: input(
                format_rational(exif.exposure_time.as_ref()),
                "例如 1/125",
                window,
                cx,
            ),
            f_number: input(
                format_rational(exif.f_number.as_ref()),
                "例如 2.8",
                window,
                cx,
            ),
            iso: input(
                exif.isospeed_ratings
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                "例如 100",
                window,
                cx,
            ),
            exposure_bias: input(
                format_rational(exif.exposure_bias_value.as_ref()),
                "例如 -1/3",
                window,
                cx,
            ),
            focal_length: input(
                format_rational(exif.focal_length.as_ref()),
                "例如 50",
                window,
                cx,
            ),
            focal_length_35mm: input(
                exif.focal_length_in35mm_film
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                "例如 75",
                window,
                cx,
            ),
            white_balance: input(
                exif.white_balance_mode
                    .map(|value| value.to_string())
                    .unwrap_or_default(),
                "0 为自动，1 为手动",
                window,
                cx,
            ),
            gps: input(
                exif.gps_info
                    .as_ref()
                    .map(GPSInfo::to_iso6709)
                    .unwrap_or_default(),
                "例如 +22.54290+114.05960/",
                window,
                cx,
            ),
        }
    }
}

pub(super) struct ExifEditor {
    photo_id: PhotoId,
    app: Entity<AppView>,
    original: ExifInfo,
    inputs: ExifInputs,
    feedback: Option<SharedString>,
}

impl ExifEditor {
    fn new(
        photo_id: PhotoId,
        exif: ExifInfo,
        app: Entity<AppView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let inputs = ExifInputs::new(&exif, window, cx);
        Self {
            photo_id,
            app,
            original: exif,
            inputs,
            feedback: None,
        }
    }

    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let exif = match self.parse(cx) {
            Ok(exif) => exif,
            Err(message) => {
                self.feedback = Some(message.into());
                cx.notify();
                return;
            }
        };

        let applied = self.app.update(cx, |app, cx| {
            app.replace_photo_exif(self.photo_id, exif, cx)
        });
        if !applied {
            self.feedback = Some("这张照片已从队列中移除".into());
            cx.notify();
            return;
        }

        window.close_sheet(cx);
        window.push_notification(Notification::success("EXIF 修改已应用到预览和导出"), cx);
    }

    fn parse(&self, cx: &App) -> Result<ExifInfo, String> {
        let created_time_text = value(&self.inputs.created_time, cx);
        let original_created_time = format_created_time(self.original.created_time);
        let created_time = if created_time_text.trim() == original_created_time {
            self.original.created_time
        } else {
            parse_created_time(&created_time_text)?
        };

        let gps_text = value(&self.inputs.gps, cx);
        let original_gps = self
            .original
            .gps_info
            .as_ref()
            .map(GPSInfo::to_iso6709)
            .unwrap_or_default();
        let gps_info = if gps_text.trim() == original_gps {
            self.original.gps_info.clone()
        } else {
            parse_gps(&gps_text)?
        };

        Ok(ExifInfo {
            created_time,
            make: optional_text(&value(&self.inputs.make, cx)),
            model: optional_text(&value(&self.inputs.model, cx)),
            exposure_time: parse_rational(&value(&self.inputs.exposure_time, cx), "快门")?,
            f_number: parse_rational(&value(&self.inputs.f_number, cx), "光圈")?,
            isospeed_ratings: parse_integer(&value(&self.inputs.iso, cx), "ISO")?,
            exposure_bias_value: parse_rational(
                &value(&self.inputs.exposure_bias, cx),
                "曝光补偿",
            )?,
            focal_length: parse_rational(&value(&self.inputs.focal_length, cx), "实际焦距")?,
            white_balance_mode: parse_integer(
                &value(&self.inputs.white_balance, cx),
                "白平衡模式",
            )?,
            focal_length_in35mm_film: parse_integer(
                &value(&self.inputs.focal_length_35mm, cx),
                "等效焦距",
            )?,
            lens_make: optional_text(&value(&self.inputs.lens_make, cx)),
            lens_model: optional_text(&value(&self.inputs.lens_model, cx)),
            gps_info,
        })
    }

    fn render_input(
        &self,
        label: &'static str,
        input: &Entity<InputState>,
        cx: &App,
    ) -> impl IntoElement {
        field(label, Input::new(input).w_full(), cx)
    }
}

impl Render for ExifEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_6()
            .pb_4()
            .child(hint(
                "修改后的值只覆盖当前队列照片，并用于 EXIF 文字水印的预览和导出；不会改写原始文件。清空输入框即可移除对应值。",
                cx,
            ))
            .when_some(self.feedback.clone(), |this, feedback| {
                this.child(warning(feedback, cx))
            })
            .child(
                GroupBox::new()
                    .id("exif-camera-fields")
                    .w_full()
                    .title("相机与时间")
                    .child(self.render_input("拍摄日期", &self.inputs.created_time, cx))
                    .child(self.render_input("相机品牌", &self.inputs.make, cx))
                    .child(self.render_input("相机型号", &self.inputs.model, cx))
                    .child(self.render_input("镜头品牌", &self.inputs.lens_make, cx))
                    .child(self.render_input("镜头型号", &self.inputs.lens_model, cx)),
            )
            .child(
                GroupBox::new()
                    .id("exif-exposure-fields")
                    .w_full()
                    .title("曝光参数")
                    .child(self.render_input("快门", &self.inputs.exposure_time, cx))
                    .child(self.render_input("光圈", &self.inputs.f_number, cx))
                    .child(self.render_input("ISO", &self.inputs.iso, cx))
                    .child(self.render_input("曝光补偿", &self.inputs.exposure_bias, cx))
                    .child(self.render_input("实际焦距（mm）", &self.inputs.focal_length, cx))
                    .child(self.render_input(
                        "等效焦距（mm）",
                        &self.inputs.focal_length_35mm,
                        cx,
                    ))
                    .child(self.render_input("白平衡模式", &self.inputs.white_balance, cx)),
            )
            .child(
                GroupBox::new()
                    .id("exif-location-fields")
                    .w_full()
                    .title("位置")
                    .child(self.render_input("GPS（ISO 6709）", &self.inputs.gps, cx)),
            )
    }
}

impl AppView {
    pub(super) fn open_exif_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(photo) = self.selected_photo() else {
            return;
        };
        let photo_id = photo.id();
        let exif = photo.exif().cloned().unwrap_or_default();
        let file_name: SharedString = photo
            .path()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| photo.path().to_string_lossy().into_owned())
            .into();
        let app = cx.entity();
        let editor = cx.new(|cx| ExifEditor::new(photo_id, exif, app, window, cx));

        window.open_sheet(cx, move |sheet, _, _| {
            let apply_editor = editor.clone();
            sheet
                .title(format!("EXIF 信息 · {file_name}"))
                .size(px(520.))
                .child(editor.clone())
                .footer(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("exif-cancel")
                                .label("取消")
                                .outline()
                                .on_click(|_, window, cx| window.close_sheet(cx)),
                        )
                        .child(Button::new("exif-apply").label("应用").primary().on_click(
                            move |_, window, cx| {
                                apply_editor.update(cx, |editor, cx| editor.apply(window, cx));
                            },
                        )),
                )
        });
    }

    fn replace_photo_exif(
        &mut self,
        photo_id: PhotoId,
        exif: ExifInfo,
        cx: &mut Context<Self>,
    ) -> bool {
        let is_selected = self.workspace.selected_id() == Some(photo_id);
        let Some(photo) = self.workspace.photo_mut(photo_id) else {
            return false;
        };
        photo.replace_exif(exif);
        if is_selected {
            self.refresh_preview(cx);
        }
        cx.notify();
        true
    }
}

fn input(
    value: String,
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut Context<ExifEditor>,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(value)
            .placeholder(placeholder)
    })
}

fn value(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().to_string()
}

fn optional_text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn format_created_time(value: Option<ExifDateTime>) -> String {
    value
        .map(|value| value.into_naive().format(DATE_FORMAT).to_string())
        .unwrap_or_default()
}

fn parse_created_time(value: &str) -> Result<Option<ExifDateTime>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    for format in [DATE_FORMAT, "%Y:%m:%d %H:%M:%S"] {
        if let Ok(value) = NaiveDateTime::parse_from_str(value, format) {
            return Ok(Some(ExifDateTime::Naive(value)));
        }
    }
    Err("拍摄日期格式无效，请使用 YYYY-MM-DD HH:MM:SS".to_owned())
}

fn format_rational(value: Option<&Rational>) -> String {
    value.map(ToString::to_string).unwrap_or_default()
}

fn parse_rational(value: &str, label: &str) -> Result<Option<Rational>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if let Some((numerator, denominator)) = value.split_once('/') {
        let numerator = numerator
            .trim()
            .parse::<i32>()
            .map_err(|_| format!("{label}格式无效，请输入小数或分数"))?;
        let denominator = denominator
            .trim()
            .parse::<i32>()
            .map_err(|_| format!("{label}格式无效，请输入小数或分数"))?;
        if denominator == 0 {
            return Err(format!("{label}的分母不能为 0"));
        }
        return Ok(Some(Rational::Fraction(numerator, denominator)));
    }
    let value = value
        .parse::<f64>()
        .map_err(|_| format!("{label}格式无效，请输入小数或分数"))?;
    if !value.is_finite() {
        return Err(format!("{label}必须是有限数值"));
    }
    Ok(Some(Rational::Float(value)))
}

fn parse_integer<T>(value: &str, label: &str) -> Result<Option<T>, String>
where
    T: std::str::FromStr,
{
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    value
        .parse::<T>()
        .map(Some)
        .map_err(|_| format!("{label}必须是非负整数"))
}

fn parse_gps(value: &str) -> Result<Option<GPSInfo>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    value
        .parse::<GPSInfo>()
        .map(Some)
        .map_err(|_| "GPS 格式无效，请使用 ISO 6709，例如 +22.54290+114.05960/".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rational_and_optional_values() {
        assert!(matches!(
            parse_rational("1/125", "快门").unwrap(),
            Some(Rational::Fraction(1, 125))
        ));
        assert!(matches!(
            parse_rational("2.8", "光圈").unwrap(),
            Some(Rational::Float(value)) if (value - 2.8).abs() < f64::EPSILON
        ));
        assert!(parse_rational("", "光圈").unwrap().is_none());
        assert!(parse_rational("1/0", "快门").is_err());

        let displayed_bias = format_rational(Some(&Rational::Float(-1.0 / 3.0)));
        assert_eq!(displayed_bias, "-0.33");
        assert!(parse_rational(&displayed_bias, "曝光补偿").is_ok());
    }

    #[test]
    fn parses_supported_date_formats() {
        assert!(parse_created_time("2026-09-24 18:30:00").unwrap().is_some());
        assert!(parse_created_time("2026:09:24 18:30:00").unwrap().is_some());
        assert!(parse_created_time("2026/09/24").is_err());
    }
}
