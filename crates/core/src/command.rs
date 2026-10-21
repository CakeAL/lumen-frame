//! 可序列化的应用操作；传输协议、鉴权及线程调度由 MCP/桌面适配器负责。
use super::{Session, export::ExportScope, metadata::ExifData};
use crate::{
    persistence::{presets::WatermarkPreset, settings::AppSettings},
    rotation::Rotation,
    watermark::{PaletteTheme, TextGroup, TextParams},
    workspace::PhotoId,
};
use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{path::PathBuf, time::Duration};

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    #[serde(default)]
    pub expected_revision: Option<u64>,
    pub command: Command,
}

/// 配置使用现有水印模型：所有背景、边框、阴影、富文本和色卡参数均可调用。
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "method",
    content = "params",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Command {
    GetState,
    ImportPhotos {
        paths: Vec<PathBuf>,
    },
    SelectPhoto {
        photo_id: PhotoId,
    },
    RemovePhoto {
        photo_id: PhotoId,
    },
    ClearPhotos,
    GetWatermark {
        photo_id: Option<PhotoId>,
    },
    SetWatermark {
        watermark: Box<WatermarkPreset>,
        all_photos: bool,
    },
    PatchParams {
        changes: Map<String, Value>,
    },
    SetCustomText {
        text: String,
    },
    SetEnvironment {
        output_folder: Option<PathBuf>,
        jpeg_quality: i32,
        default_font: String,
        rotation: Rotation,
    },
    ResetWatermark,
    SetTextGroups {
        groups: Vec<TextGroup>,
    },
    AddTextGroup {
        group: TextGroup,
    },
    UpdateTextGroup {
        group_ix: usize,
        group: TextGroup,
    },
    RemoveTextGroup {
        group_ix: usize,
    },
    AddTextLine {
        group_ix: usize,
        template: String,
        style: TextParams,
    },
    UpdateTextLine {
        group_ix: usize,
        line_ix: usize,
        template: String,
        style: TextParams,
    },
    RemoveTextLine {
        group_ix: usize,
        line_ix: usize,
    },
    FormatTextLine {
        group_ix: usize,
        line_ix: usize,
        start: usize,
        end: usize,
        change: crate::watermark::TextStyleChange,
    },
    EditTextLine {
        group_ix: usize,
        line_ix: usize,
        template: String,
    },
    ApplyCurrentToAll,
    GetExif {
        photo_id: PhotoId,
    },
    UpdateExif {
        photo_id: PhotoId,
        changes: Map<String, Value>,
    },
    SetGps {
        photo_id: PhotoId,
        latitude: f64,
        longitude: f64,
    },
    ExtractPalette {
        photo_id: PhotoId,
        count: usize,
        theme: PaletteTheme,
    },
    RenderPreview {
        photo_id: PhotoId,
        max_edge: i32,
        output: PathBuf,
    },
    RenderThumbnail {
        photo_id: PhotoId,
        output: PathBuf,
    },
    ExportPhotos {
        all_photos: bool,
    },
    ListPresets,
    SavePreset {
        name: String,
    },
    LoadPreset {
        name: String,
        all_photos: bool,
    },
    DeletePreset {
        name: String,
    },
    GetSettings,
    SaveSettings {
        settings: AppSettings,
    },
    ListLogos,
    ImportLogos {
        paths: Vec<PathBuf>,
        #[serde(default)]
        names: Vec<Option<String>>,
    },
    DeleteLogo {
        logo_id: u64,
    },
    ExportGainmap {
        input: PathBuf,
        output: PathBuf,
    },
    ExportColourGainmap {
        input: PathBuf,
        output: PathBuf,
        rotation: Rotation,
    },
    InspectVideo {
        ffmpeg: PathBuf,
        input: PathBuf,
    },
    ExportMotionPhoto {
        ffmpeg: PathBuf,
        input: PathBuf,
        output: PathBuf,
        start_seconds: f64,
        end_seconds: f64,
        cover_seconds: f64,
        jpeg_quality: u8,
        rotation: Rotation,
        max_output_size: u64,
    },
    ClearMapCache,
    SearchPlaces {
        query: String,
    },
    DetectFfmpeg,
    ValidateFfmpeg {
        path: PathBuf,
    },
    RenderGainmapPreview {
        input: PathBuf,
        show_gainmap: bool,
        output: PathBuf,
    },
    RenderColourGainmapPreview {
        input: PathBuf,
        rotation: Rotation,
        output: PathBuf,
        gainmap_output: PathBuf,
    },
    RenderMotionPhotoCover {
        ffmpeg: PathBuf,
        input: PathBuf,
        output: PathBuf,
        start_seconds: f64,
        end_seconds: f64,
        cover_seconds: f64,
        rotation: Rotation,
    },
    CheckUpdate,
    InstallUpdate,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    Conflict,
    OperationFailed,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationError {
    pub code: ErrorCode,
    pub message: String,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reply {
    pub revision: u64,
    pub result: std::result::Result<CommandOutput, OperationError>,
}
/// 内容由具体命令确定，统一 JSON 结果不包含 GPUI 或 libvips 句柄。
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CommandOutput(Value);
impl CommandOutput {
    pub fn value(&self) -> &Value {
        &self.0
    }
}

impl Session {
    pub fn dispatch(&mut self, request: Request) -> Reply {
        let result = if request
            .expected_revision
            .is_some_and(|r| r != self.revision())
        {
            Err(OperationError {
                code: ErrorCode::Conflict,
                message: "状态已更新，请重新读取后再提交".into(),
            })
        } else {
            self.execute(request.command).map_err(|e| OperationError {
                code: ErrorCode::OperationFailed,
                message: format!("{e:#}"),
            })
        };
        Reply {
            revision: self.revision(),
            result,
        }
    }
    pub fn dispatch_json(&mut self, request: Value) -> Reply {
        match serde_json::from_value(request) {
            Ok(request) => self.dispatch(request),
            Err(error) => Reply {
                revision: self.revision(),
                result: Err(OperationError {
                    code: ErrorCode::InvalidRequest,
                    message: error.to_string(),
                }),
            },
        }
    }
    /// 阻塞 I/O 必须由调用方放在后台运行；UI 使用同一模块中的冻结任务而非阻塞主线程。
    pub fn execute(&mut self, command: Command) -> Result<CommandOutput> {
        use Command::*;
        let value = match command {
            GetState => {
                json!({ "selected":self.workspace().selected_id(), "photos":self.workspace().photos().iter().map(|p| json!({"id":p.id(),"path":p.path(),"exif_loaded":p.exif().is_some()})).collect::<Vec<_>>(), "current": watermark_output(self.current()) })
            }
            ImportPhotos { paths } => {
                let scanned = super::import::scan_photo_paths(paths);
                let ids = self
                    .add_photos(scanned.paths)
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect::<Vec<_>>();
                for id in &ids {
                    let path = self.workspace().photo(*id).unwrap().path().to_path_buf();
                    self.set_loaded_exif(*id, crate::media::ExifInfo::read(&path).ok());
                }
                json!({"added":ids,"errors":scanned.errors})
            }
            SelectPhoto { photo_id } => {
                self.workspace().photo(photo_id).context("照片不存在")?;
                self.select(photo_id);
                json!({"selected":photo_id})
            }
            RemovePhoto { photo_id } => {
                self.workspace().photo(photo_id).context("照片不存在")?;
                self.remove_photo(photo_id);
                json!({"removed":photo_id})
            }
            ClearPhotos => json!({"removed":self.clear()}),
            GetWatermark { photo_id } => watermark_output(&if let Some(id) = photo_id {
                self.watermark_for(id)?
            } else {
                self.current().clone()
            }),
            SetWatermark {
                watermark,
                all_photos,
            } => {
                let watermark = *watermark;
                validate_watermark(&watermark)?;
                if all_photos {
                    self.apply_to_all(watermark.clone());
                }
                let mut watermark = watermark;
                watermark.params.custom_text = self.params().custom_text.clone();
                self.replace_current(watermark);
                json!(true)
            }
            PatchParams { changes } => {
                let mut value = serde_json::to_value(self.params())?;
                merge_params(&mut value, Value::Object(changes), "params")?;
                let mut watermark = self.current().clone();
                watermark.params = serde_json::from_value(value)?;
                watermark.params.custom_text = self.params().custom_text.clone();
                validate_watermark(&watermark)?;
                let rotation = watermark.params.rotation;
                self.replace_current(watermark);
                self.params_mut().rotation = rotation;
                json!(true)
            }
            SetCustomText { text } => {
                self.params_mut().custom_text = text;
                json!(true)
            }
            SetEnvironment {
                output_folder,
                jpeg_quality,
                default_font,
                rotation,
            } => {
                ensure!(
                    (1..=100).contains(&jpeg_quality),
                    "JPEG 质量必须在 1 至 100 之间"
                );
                let params = self.params_mut();
                params.output_folder = output_folder;
                params.quality = jpeg_quality;
                params.default_font = default_font;
                params.rotation = rotation;
                json!(true)
            }
            ResetWatermark => {
                let mut watermark = WatermarkPreset {
                    params: Default::default(),
                    text_groups: vec![TextGroup::default()],
                };
                watermark.params.custom_text = self.params().custom_text.clone();
                self.replace_current(watermark);
                json!(true)
            }
            SetTextGroups { groups } => {
                let mut w = self.current().clone();
                w.text_groups = groups;
                validate_watermark(&w)?;
                self.replace_current(w);
                json!(true)
            }
            AddTextGroup { group } => {
                let mut w = self.current().clone();
                w.text_groups.push(group);
                validate_watermark(&w)?;
                let ix = w.text_groups.len() - 1;
                self.replace_current(w);
                json!({"group_ix":ix})
            }
            UpdateTextGroup { group_ix, group } => {
                let mut w = self.current().clone();
                *w.text_groups.get_mut(group_ix).context("文字组不存在")? = group;
                validate_watermark(&w)?;
                self.replace_current(w);
                json!(true)
            }
            RemoveTextGroup { group_ix } => {
                let mut w = self.current().clone();
                ensure!(group_ix < w.text_groups.len(), "文字组不存在");
                w.text_groups.remove(group_ix);
                for group in &mut w.text_groups {
                    repair_attachment(&mut group.attachment, group_ix);
                }
                let mut attachment = w.params.palette_card.attachment().cloned();
                repair_attachment(&mut attachment, group_ix);
                w.params.palette_card.set_attachment(attachment);
                self.replace_current(w);
                json!(true)
            }
            AddTextLine {
                group_ix,
                template,
                style,
            } => {
                let mut w = self.current().clone();
                let g = w.text_groups.get_mut(group_ix).context("文字组不存在")?;
                g.text.template.push(template);
                g.text.text_params.push(style);
                validate_watermark(&w)?;
                self.replace_current(w);
                json!(true)
            }
            UpdateTextLine {
                group_ix,
                line_ix,
                template,
                style,
            } => {
                let mut w = self.current().clone();
                let g = w.text_groups.get_mut(group_ix).context("文字组不存在")?;
                *g.text.template.get_mut(line_ix).context("文字行不存在")? = template;
                *g.text
                    .text_params
                    .get_mut(line_ix)
                    .context("文字行不存在")? = style;
                validate_watermark(&w)?;
                self.replace_current(w);
                json!(true)
            }
            RemoveTextLine { group_ix, line_ix } => {
                let mut w = self.current().clone();
                let g = w.text_groups.get_mut(group_ix).context("文字组不存在")?;
                ensure!(line_ix < g.text.template.len(), "文字行不存在");
                g.text.template.remove(line_ix);
                g.text.text_params.remove(line_ix);
                self.replace_current(w);
                json!(true)
            }
            FormatTextLine {
                group_ix,
                line_ix,
                start,
                end,
                change,
            } => {
                let mut w = self.current().clone();
                let g = w.text_groups.get_mut(group_ix).context("文字组不存在")?;
                let text = g.text.template.get(line_ix).context("文字行不存在")?;
                ensure!(start <= end && end <= text.len(), "选区超出文字范围");
                let style = g
                    .text
                    .text_params
                    .get_mut(line_ix)
                    .context("文字行样式不存在")?;
                super::text::format_line(text, style, start..end, change);
                self.replace_current(w);
                json!(true)
            }
            EditTextLine {
                group_ix,
                line_ix,
                template,
            } => {
                let mut w = self.current().clone();
                let g = w.text_groups.get_mut(group_ix).context("文字组不存在")?;
                let text = g.text.template.get_mut(line_ix).context("文字行不存在")?;
                let style = g
                    .text
                    .text_params
                    .get_mut(line_ix)
                    .context("文字行样式不存在")?;
                crate::watermark::rich_text::rebase_spans(text, &template, &mut style.spans);
                *text = template;
                self.replace_current(w);
                json!(true)
            }
            ApplyCurrentToAll => {
                self.apply_to_all(self.current().clone());
                json!(true)
            }
            GetExif { photo_id } => serde_json::to_value(ExifData::from(
                self.workspace()
                    .photo(photo_id)
                    .context("照片不存在")?
                    .exif()
                    .unwrap_or(&Default::default()),
            ))?,
            UpdateExif { photo_id, changes } => {
                let original = self
                    .workspace()
                    .photo(photo_id)
                    .context("照片不存在")?
                    .exif()
                    .cloned()
                    .unwrap_or_default();
                self.replace_exif(photo_id, ExifData::apply_patch(&original, changes)?)?;
                json!(true)
            }
            SetGps {
                photo_id,
                latitude,
                longitude,
            } => {
                let mut exif = self
                    .workspace()
                    .photo(photo_id)
                    .context("照片不存在")?
                    .exif()
                    .cloned()
                    .unwrap_or_default();
                exif.gps_info =
                    Some(super::geolocation::Location::new(latitude, longitude)?.to_gps()?);
                self.replace_exif(photo_id, exif)?;
                json!(true)
            }
            ExtractPalette {
                photo_id,
                count,
                theme,
            } => {
                let photo = self.workspace().photo(photo_id).context("照片不存在")?;
                json!(
                    super::photo_palette::extract_with_theme(photo.path(), count, theme)?
                        .iter()
                        .map(|s| json!({"rgb":s.rgb(),"ratio":s.ratio()}))
                        .collect::<Vec<_>>()
                )
            }
            RenderPreview {
                photo_id,
                max_edge,
                output,
            } => {
                let photo = self.workspace().photo(photo_id).context("照片不存在")?;
                let watermark = self.watermark_for(photo_id)?;
                let image = super::image::render_preview(&super::image::PreviewJob {
                    path: photo.path().to_path_buf(),
                    exif: photo.exif().cloned(),
                    params: watermark.params,
                    text_groups: watermark.text_groups,
                    max_edge,
                })?;
                image.save(&output)?;
                json!({"path":output,"width":image.width(),"height":image.height()})
            }
            RenderThumbnail { photo_id, output } => {
                let image = super::image::render_thumbnail(
                    self.workspace()
                        .photo(photo_id)
                        .context("照片不存在")?
                        .path(),
                )?;
                image.save(&output)?;
                json!({"path":output})
            }
            ExportPhotos { all_photos } => {
                let jobs = self.export_jobs(if all_photos {
                    ExportScope::AllPhotos
                } else {
                    ExportScope::CurrentPhoto
                });
                ensure!(!jobs.is_empty(), "没有可导出的照片");
                let mut outputs = Vec::new();
                let mut errors = Vec::new();
                for job in jobs {
                    let path = job.path.clone();
                    match job.run() {
                        Ok(()) => outputs.push(path),
                        Err(e) => errors.push(json!({"input":path,"message":format!("{e:#}")})),
                    }
                }
                json!({"exported_inputs":outputs,"errors":errors})
            }
            ListPresets => {
                json!({"builtin":super::presets::builtin()?.into_iter().map(|(name,_)|name).collect::<Vec<_>>(),"personal":super::presets::list_in(self.storage().presets()?)?})
            }
            SavePreset { name } => {
                ensure!(!super::presets::is_builtin(name.trim()), "内置预设不能覆盖");
                json!({"path":super::presets::save_in(self.storage().presets()?,&name,self.current())?})
            }
            LoadPreset { name, all_photos } => {
                let preset = if super::presets::is_builtin(&name) {
                    super::presets::load_builtin(&name)?
                } else {
                    super::presets::load_in(self.storage().presets()?, &name)?
                };
                self.apply_preset(preset, all_photos);
                json!(true)
            }
            DeletePreset { name } => {
                ensure!(!super::presets::is_builtin(name.trim()), "内置预设不能删除");
                super::presets::delete_in(self.storage().presets()?, &name)?;
                json!(true)
            }
            GetSettings => {
                serde_json::to_value(super::settings::load_at(self.storage().settings()?))?
            }
            SaveSettings { settings } => {
                ensure!(
                    (1..=100).contains(&settings.jpeg_quality),
                    "JPEG 质量必须在 1 至 100 之间"
                );
                super::settings::save_at(self.storage().settings()?, &settings)?;
                let params = self.params_mut();
                params.output_folder = settings.output_folder;
                params.quality = settings.jpeg_quality;
                params.default_font = settings.default_font;
                json!(true)
            }
            ListLogos => logos_output(super::logos::library_in(self.storage().logos()?)?),
            ImportLogos { paths, names } => {
                ensure!(
                    names.is_empty() || names.len() == paths.len(),
                    "Logo 名称数量必须与文件数量一致，或省略名称"
                );
                let files = paths
                    .into_iter()
                    .enumerate()
                    .map(|(ix, path)| {
                        super::logos::LogoImport::new(path, names.get(ix).cloned().flatten())
                    })
                    .collect::<Vec<_>>();
                let assets = super::logos::import_named_in(self.storage().logos()?, &files)?;
                self.params_mut().custom_logos = super::logos::snapshot(&assets);
                logos_output(super::logos::library_in(self.storage().logos()?)?)
            }
            DeleteLogo { logo_id } => {
                let assets = super::logos::delete_in(self.storage().logos()?, logo_id)?;
                self.params_mut().custom_logos = super::logos::snapshot(&assets);
                logos_output(super::logos::library_in(self.storage().logos()?)?)
            }
            ExportGainmap { input, output } => {
                json!({"exported":super::image::export_gainmap(&input,&output)?,"path":output})
            }
            ExportColourGainmap {
                input,
                output,
                rotation,
            } => {
                super::image::export_colour_gainmap(&input, &output, rotation)?;
                json!({"path":output})
            }
            InspectVideo { ffmpeg, input } => {
                let info = super::motion_photo::inspect_motion_photo_video(&ffmpeg, &input)?;
                json!({"duration_seconds":info.duration.as_secs_f64(),"width":info.width,"height":info.height})
            }
            ExportMotionPhoto {
                ffmpeg,
                input,
                output,
                start_seconds,
                end_seconds,
                cover_seconds,
                jpeg_quality,
                rotation,
                max_output_size,
            } => {
                for time in [start_seconds, end_seconds, cover_seconds] {
                    ensure!(
                        time.is_finite() && (0.0..=86400.0).contains(&time),
                        "视频时间必须在 0 至 86400 秒之间"
                    );
                }
                super::motion_photo::export_motion_photo(
                    &super::motion_photo::MotionPhotoOptions {
                        ffmpeg_path: &ffmpeg,
                        video_path: &input,
                        output_path: &output,
                        start: Duration::from_secs_f64(start_seconds),
                        end: Duration::from_secs_f64(end_seconds),
                        cover_time: Duration::from_secs_f64(cover_seconds),
                        jpeg_quality,
                        rotation,
                        max_output_size,
                    },
                )?;
                json!({"path":output})
            }
            ClearMapCache => {
                super::geolocation::clear_tile_cache()?;
                json!(true)
            }
            SearchPlaces { query } => {
                let settings = super::settings::load_at(self.storage().settings()?);
                let places =
                    super::geolocation::MapService::from_settings(&settings.map)?.search(&query)?;
                json!(places.iter().map(|p|json!({"label":p.label,"latitude":p.location.latitude(),"longitude":p.location.longitude()})).collect::<Vec<_>>())
            }
            DetectFfmpeg => json!({"path":super::motion_photo::detect_ffmpeg()?}),
            ValidateFfmpeg { path } => {
                super::motion_photo::validate_ffmpeg(&path)?;
                json!(true)
            }
            RenderGainmapPreview {
                input,
                show_gainmap,
                output,
            } => {
                let image = super::image::render_gainmap_preview(&input, show_gainmap)?;
                if let Some(image) = image {
                    image.save(&output)?;
                    json!({"path":output})
                } else {
                    Value::Null
                }
            }
            RenderColourGainmapPreview {
                input,
                rotation,
                output,
                gainmap_output,
            } => {
                let (base, map) = super::image::render_colour_gainmap_preview(&input, rotation)?;
                base.save(&output)?;
                map.save(&gainmap_output)?;
                json!({"path":output,"gainmap_path":gainmap_output})
            }
            RenderMotionPhotoCover {
                ffmpeg,
                input,
                output,
                start_seconds,
                end_seconds,
                cover_seconds,
                rotation,
            } => {
                for time in [start_seconds, end_seconds, cover_seconds] {
                    ensure!(
                        time.is_finite() && (0.0..=86400.0).contains(&time),
                        "视频时间必须在 0 至 86400 秒之间"
                    );
                }
                let image = super::image::render_motion_photo_cover(
                    &ffmpeg,
                    &input,
                    Duration::from_secs_f64(start_seconds),
                    Duration::from_secs_f64(end_seconds),
                    Duration::from_secs_f64(cover_seconds),
                    rotation,
                )?;
                image.save(&output)?;
                json!({"path":output})
            }
            CheckUpdate => match super::update::check_now_and_record()? {
                super::update::UpdateAvailability::UpToDate => json!({"available":false}),
                super::update::UpdateAvailability::Available { version } => {
                    json!({"available":true,"version":version})
                }
            },
            InstallUpdate => json!({"version":super::update::install_latest()?}),
        };
        Ok(CommandOutput(value))
    }
}
fn logos_output(assets: Vec<super::logos::LogoAsset>) -> Value {
    json!(
        assets
            .iter()
            .map(|a| json!({"id":a.id(),"name":a.name(),"path":a.path(),"token":a.token(),"builtin":a.is_builtin()}))
            .collect::<Vec<_>>()
    )
}

fn watermark_output(watermark: &WatermarkPreset) -> Value {
    json!({"watermark":watermark,"custom_text":watermark.params.custom_text,
        "environment":{"output_folder":watermark.params.output_folder,"jpeg_quality":watermark.params.quality,
            "default_font":watermark.params.default_font,"rotation":watermark.params.rotation}})
}

fn merge_params(target: &mut Value, patch: Value, path: &str) -> Result<()> {
    if let (Some(fields), Value::Object(changes)) = (target.as_object_mut(), &patch) {
        for (key, patch) in changes {
            let field = fields
                .get_mut(key)
                .with_context(|| format!("未知或只读的水印参数：{path}.{key}"))?;
            merge_params(field, patch.clone(), &format!("{path}.{key}"))?;
        }
    } else {
        *target = patch;
    }
    Ok(())
}
pub(crate) fn repair_attachment(
    attachment: &mut Option<crate::watermark::GroupAttachment>,
    removed: usize,
) {
    if let Some(a) = attachment {
        if a.target == removed {
            *attachment = None;
        } else if a.target > removed {
            a.target -= 1;
        }
    }
}
pub(crate) fn validate_watermark(w: &WatermarkPreset) -> Result<()> {
    let p = &w.params;
    for value in [
        p.border_ratio.0,
        p.border_ratio.1,
        p.border_ratio.2,
        p.border_ratio.3,
    ] {
        ensure!(
            value.is_finite() && (0.0..=2.0).contains(&value),
            "画布边距必须在 0 至 200% 之间"
        );
    }
    ensure!(
        p.shadow_density.is_finite() && (0.0..=4.0).contains(&p.shadow_density),
        "阴影浓度必须在 0 至 4 之间"
    );
    ensure!(
        p.shadow_size.is_finite() && (0.0..=1.0).contains(&p.shadow_size),
        "阴影大小无效"
    );
    ensure!(
        p.border_radius.is_finite() && (0.0..=0.5).contains(&p.border_radius),
        "照片圆角无效"
    );
    ensure!(
        p.blur_sigma.is_finite() && (0.0..=1000.0).contains(&p.blur_sigma),
        "背景模糊参数无效"
    );
    ensure!(
        p.grain_strength.is_finite() && (0.0..=1.0).contains(&p.grain_strength),
        "背景颗粒强度必须在 0 至 100% 之间"
    );
    if let Some((a, b)) = p.aspect_ratio {
        ensure!(
            a.is_finite() && b.is_finite() && a > 0.0 && b > 0.0,
            "宽高比必须大于 0"
        );
    }
    for (ix, group) in w.text_groups.iter().enumerate() {
        ensure!(
            group.text.template.len() == group.text.text_params.len(),
            "文字行和样式数量不一致"
        );
        ensure!(
            group.padding.is_finite() && group.padding >= 0.0,
            "文字组留白无效"
        );
        let mut seen = vec![ix];
        let mut next = group.attachment.as_ref();
        while let Some(a) = next {
            ensure!(
                a.target < w.text_groups.len() && a.gap.is_finite() && a.gap >= 0.0,
                "依附目标或间距无效"
            );
            ensure!(!seen.contains(&a.target), "文字组依附关系不能形成循环");
            seen.push(a.target);
            next = w.text_groups[a.target].attachment.as_ref();
        }
        for (text, style) in group.text.template.iter().zip(&group.text.text_params) {
            ensure!(
                style.size.is_finite()
                    && style.size > 0.0
                    && style.line_spacing.is_finite()
                    && style.line_spacing > 0.0,
                "文字大小或行距无效"
            );
            for span in &style.spans {
                ensure!(
                    span.start <= span.end
                        && span.end <= text.len()
                        && text.is_char_boundary(span.start)
                        && text.is_char_boundary(span.end),
                    "富文本范围必须位于 UTF-8 字符边界"
                );
            }
        }
    }
    if let Some(a) = p.palette_card.attachment() {
        ensure!(
            a.target < w.text_groups.len() && a.gap.is_finite() && a.gap >= 0.0,
            "色卡依附目标或间距无效"
        );
    }
    Ok(())
}
