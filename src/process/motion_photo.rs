//! Google Motion Photo 导出。视频处理统一委托给 FFmpeg；本模块仅拼接 JPEG/XMP。
use anyhow::{Context, Result, anyhow, ensure};
use ffmpeg_sidecar::command::FfmpegCommand;
use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

const MAX_DURATION: Duration = Duration::from_secs(10);
pub const DEFAULT_MAX_OUTPUT_SIZE: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct MotionPhotoVideoInfo {
    pub duration: Duration,
    pub width: u16,
    pub height: u16,
}

/// 在终端 PATH 中自动检测 FFmpeg。
pub fn detect_ffmpeg() -> Result<PathBuf> {
    let mut command = FfmpegCommand::new();
    command.arg("-version");
    let output = command
        .as_inner_mut()
        .output()
        .context("无法在终端 PATH 中启动 ffmpeg")?;
    ensure!(output.status.success(), "终端中的 ffmpeg 无法运行");
    Ok(PathBuf::from("ffmpeg"))
}
pub fn validate_ffmpeg(path: &Path) -> Result<()> {
    let mut command = FfmpegCommand::new_with_path(path);
    command.arg("-version");
    let output = command
        .as_inner_mut()
        .output()
        .with_context(|| format!("无法启动 FFmpeg：{}", path.display()))?;
    ensure!(output.status.success(), "指定的 FFmpeg 无法运行");
    Ok(())
}
pub fn inspect_motion_photo_video(ffmpeg: &Path, video: &Path) -> Result<MotionPhotoVideoInfo> {
    let output = run(
        Command::new(ffprobe_path(ffmpeg)?)
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=width,height:format=duration",
                "-of",
                "default=noprint_wrappers=1",
            ])
            .arg(video),
        "FFprobe 无法读取视频",
    )?;
    let text = String::from_utf8_lossy(&output.stdout);
    let get = |key: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(&format!("{key}=")))
            .ok_or_else(|| anyhow!("FFprobe 未返回 {key}"))
    };
    let duration = get("duration")?
        .parse::<f64>()
        .context("FFprobe 返回了无效视频时长")?;
    let width = get("width")?
        .parse()
        .context("FFprobe 返回了无效视频宽度")?;
    let height = get("height")?
        .parse()
        .context("FFprobe 返回了无效视频高度")?;
    ensure!(duration.is_finite() && duration > 0.0, "视频时长无效");
    Ok(MotionPhotoVideoInfo {
        duration: Duration::from_secs_f64(duration),
        width,
        height,
    })
}
pub fn render_motion_photo_cover(
    ffmpeg: &Path,
    video: &Path,
    start: Duration,
    end: Duration,
    cover: Duration,
) -> Result<Vec<u8>> {
    validate_range(start, end, cover)?;
    let mut command = FfmpegCommand::new_with_path(ffmpeg);
    command
        .hide_banner()
        .args(["-loglevel", "error", "-ss"])
        .arg(seconds(cover))
        .arg("-i")
        .arg(video)
        .args([
            "-map",
            "0:v:0",
            "-frames:v",
            "1",
            "-q:v",
            "2",
            "-f",
            "image2pipe",
            "-vcodec",
            "mjpeg",
            "pipe:1",
        ]);
    let output = run(command.as_inner_mut(), "FFmpeg 无法解码封面帧")?;
    ensure!(
        output.stdout.starts_with(&[0xff, 0xd8]),
        "FFmpeg 没有输出 JPEG 封面帧"
    );
    Ok(output.stdout)
}
#[derive(Debug, Clone)]
pub struct MotionPhotoOptions<'a> {
    pub ffmpeg_path: &'a Path,
    pub video_path: &'a Path,
    pub output_path: &'a Path,
    pub start: Duration,
    pub end: Duration,
    pub cover_time: Duration,
    pub jpeg_quality: u8,
    /// 最终 JPEG Motion Photo 文件的最大字节数，包含封面与内嵌 MP4。
    pub max_output_size: u64,
}
/// 以 H.264 重编码选中片段，因此任意入点、出点均可独立播放。
pub fn export_motion_photo(options: &MotionPhotoOptions<'_>) -> Result<()> {
    validate_range(options.start, options.end, options.cover_time)?;
    ensure!(
        (1..=100).contains(&options.jpeg_quality),
        "JPEG 质量必须在 1 到 100 之间"
    );
    ensure!(
        options.max_output_size > 0,
        "Motion Photo 最大文件大小必须大于 0"
    );
    let info = inspect_motion_photo_video(options.ffmpeg_path, options.video_path)?;
    ensure!(options.end <= info.duration, "所选结束时间超出视频时长");
    let cover = render_motion_photo_cover(
        options.ffmpeg_path,
        options.video_path,
        options.start,
        options.end,
        options.cover_time,
    )?;
    let temporary = temporary_video_path(options.output_path);
    let duration = options.end - options.start;
    ensure!(
        cover.len() < options.max_output_size as usize,
        "封面图片已为 {:.1} MB，超过 {:.1} MB 上限",
        cover.len() as f64 / 1024.0 / 1024.0,
        options.max_output_size as f64 / 1024.0 / 1024.0,
    );
    // 预留足够空间给 MP4 容器、XMP 与 x264 的码率波动；视频编码前就把剩余预算转为目标码率。
    let video_budget = options.max_output_size as usize - cover.len();
    let video_bitrate = ((video_budget as f64 * 8.0 / duration.as_secs_f64()) * 0.45) as u64;
    ensure!(
        video_bitrate >= 100_000,
        "可用于视频的大小预算过小，请提高最大文件大小"
    );
    let video_bitrate = video_bitrate.to_string();
    let video_budget = video_budget.to_string();
    let mut command = FfmpegCommand::new_with_path(options.ffmpeg_path);
    command
        .hide_banner()
        .args(["-loglevel", "error", "-ss"])
        .arg(seconds(options.start))
        .arg("-i")
        .arg(options.video_path)
        .arg("-t")
        .arg(seconds(duration))
        .args([
            "-map", "0:v:0", "-an", "-c:v", "libx264", "-preset", "medium",
        ])
        .args([
            "-b:v",
            &video_bitrate,
            "-maxrate",
            &video_bitrate,
            "-bufsize",
        ])
        .arg(format!(
            "{}k",
            video_bitrate.parse::<u64>().unwrap_or_default() * 2 / 1000
        ))
        .args([
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
            "-fs",
            &video_budget,
            "-y",
        ])
        .arg(&temporary);
    let encoded = run(
        command.as_inner_mut(),
        "FFmpeg 无法裁切并编码 Motion Photo 视频",
    );
    if let Err(error) = encoded {
        let _ = fs::remove_file(&temporary);
        return Err(error.context("需要带 libx264 编码器的 FFmpeg"));
    }
    let video = fs::read(&temporary).context("无法读取 FFmpeg 生成的视频片段")?;
    let _ = fs::remove_file(&temporary);
    let timestamp: u64 = options
        .cover_time
        .checked_sub(options.start)
        .unwrap_or(Duration::ZERO)
        .as_micros()
        .try_into()
        .context("封面时间戳超出元数据范围")?;
    let jpeg = insert_motion_photo_xmp(cover, video.len(), timestamp)?;
    ensure!(
        jpeg.len() + video.len() <= options.max_output_size as usize,
        "生成的 Motion Photo 为 {:.1} MB，超过 {:.1} MB 上限；请缩短片段、降低画质或提高上限",
        (jpeg.len() + video.len()) as f64 / 1024.0 / 1024.0,
        options.max_output_size as f64 / 1024.0 / 1024.0,
    );
    let mut output = BufWriter::new(
        File::create(options.output_path)
            .with_context(|| format!("无法创建输出文件：{}", options.output_path.display()))?,
    );
    output.write_all(&jpeg)?;
    output.write_all(&video)?;
    output.flush()?;
    Ok(())
}
fn validate_range(start: Duration, end: Duration, cover: Duration) -> Result<()> {
    ensure!(start < end, "结束时间必须晚于起始时间");
    ensure!(
        end - start <= MAX_DURATION,
        "Motion Photo 视频最长只能为 10 秒"
    );
    ensure!(
        (start..end).contains(&cover),
        "封面帧必须落在所选的视频区间内"
    );
    Ok(())
}
fn ffprobe_path(ffmpeg: &Path) -> Result<PathBuf> {
    if ffmpeg != Path::new("ffmpeg") {
        let name = if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        };
        if let Some(parent) = ffmpeg.parent() {
            let sibling = parent.join(name);
            if sibling.is_file() {
                return Ok(sibling);
            }
        }
    }
    Ok(PathBuf::from("ffprobe"))
}
fn run(command: &mut Command, action: &str) -> Result<std::process::Output> {
    let output = command.output().with_context(|| action.to_owned())?;
    if output.status.success() {
        return Ok(output);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(anyhow!(
        "{action}{}",
        if stderr.is_empty() {
            String::new()
        } else {
            format!("：{stderr}")
        }
    ))
}
fn seconds(value: Duration) -> String {
    format!("{:.6}", value.as_secs_f64())
}
fn temporary_video_path(output: &Path) -> PathBuf {
    let stem = output
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("motion-photo");
    output.with_file_name(format!(".{stem}.motion-photo-{}.mp4", std::process::id()))
}
fn insert_motion_photo_xmp(
    mut jpeg: Vec<u8>,
    video_size: usize,
    timestamp: u64,
) -> Result<Vec<u8>> {
    ensure!(jpeg.starts_with(&[0xff, 0xd8]), "封面不是有效 JPEG");
    const EXIF: &[u8] = b"Exif\0\0MM\0*\0\0\0\x08\0\0\0\0\0\0\0\0";
    let xmp = format!(
        "http://ns.adobe.com/xap/1.0/\0<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description xmlns:GCamera=\"http://ns.google.com/photos/1.0/camera/\" xmlns:Container=\"http://ns.google.com/photos/1.0/container/\" xmlns:Item=\"http://ns.google.com/photos/1.0/container/item/\" GCamera:MotionPhoto=\"1\" GCamera:MotionPhotoVersion=\"1\" GCamera:MotionPhotoPresentationTimestampUs=\"{timestamp}\"><Container:Directory><rdf:Seq><rdf:li rdf:parseType=\"Resource\"><Container:Item Item:Mime=\"image/jpeg\" Item:Semantic=\"Primary\"/></rdf:li><rdf:li rdf:parseType=\"Resource\"><Container:Item Item:Mime=\"video/mp4\" Item:Semantic=\"MotionPhoto\" Item:Length=\"{video_size}\" Item:Padding=\"0\"/></rdf:li></rdf:Seq></Container:Directory></rdf:Description></rdf:RDF></x:xmpmeta>"
    );
    let mut segments = app1_segment(EXIF)?;
    segments.extend(app1_segment(xmp.as_bytes())?);
    jpeg.splice(2..2, segments);
    Ok(jpeg)
}
fn app1_segment(payload: &[u8]) -> Result<Vec<u8>> {
    ensure!(payload.len() + 2 <= u16::MAX as usize, "JPEG APP1 段过长");
    let mut segment = Vec::with_capacity(payload.len() + 4);
    segment.extend_from_slice(&[0xff, 0xe1]);
    segment.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    segment.extend_from_slice(payload);
    Ok(segment)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inserts_xmp() {
        let jpeg = insert_motion_photo_xmp(vec![0xff, 0xd8, 0xff, 0xd9], 123, 456).unwrap();
        assert!(
            jpeg.windows(b"GCamera:MotionPhoto=\"1\"".len())
                .any(|x| x == b"GCamera:MotionPhoto=\"1\"")
        );
    }

    #[test]
    #[ignore = "requires FFmpeg and the developer's local HEVC fixture"]
    fn exports_local_hevc_fixture_with_ffmpeg() {
        let ffmpeg = detect_ffmpeg().unwrap();
        let video =
            Path::new("/Users/cakeal/Movies/20260906广东深圳石岩气象梯度观测塔晚霞延时.mp4");
        let output = std::env::temp_dir().join("lumen-frame-motion-photo-test.jpg");
        let result = export_motion_photo(&MotionPhotoOptions {
            ffmpeg_path: &ffmpeg,
            video_path: video,
            output_path: &output,
            start: Duration::from_secs(1),
            end: Duration::from_secs(2),
            cover_time: Duration::from_secs(1),
            jpeg_quality: 92,
            max_output_size: DEFAULT_MAX_OUTPUT_SIZE,
        });
        let _ = fs::remove_file(&output);
        result.unwrap();
    }
}
