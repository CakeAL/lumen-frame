//! Google Motion Photo 工作流。视频处理统一委托给 FFmpeg。
mod container;

use anyhow::{Context, Result, anyhow, ensure};
use container::insert_motion_photo_xmp;
use ffmpeg_sidecar::command::FfmpegCommand;
use std::{
    env,
    ffi::OsStr,
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use crate::rotation::Rotation;

const MAX_DURATION: Duration = Duration::from_secs(10);
pub const DEFAULT_MAX_OUTPUT_SIZE: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct MotionPhotoVideoInfo {
    pub duration: Duration,
    pub width: u16,
    pub height: u16,
}

/// 在应用 PATH 与常见安装位置中自动检测 FFmpeg。
///
/// macOS 从 Finder 启动 `.app` 时不会继承终端 PATH，因此 Homebrew 与
/// MacPorts 路径必须显式检查。返回经验证的绝对路径，让 ffprobe 可以从
/// FFmpeg 的同级目录稳定解析。
pub fn detect_ffmpeg() -> Result<PathBuf> {
    let candidates = ffmpeg_candidates(env::var_os("PATH").as_deref());
    let mut rejected = Vec::new();

    for candidate in candidates {
        if !candidate.is_file() {
            continue;
        }
        match validate_ffmpeg(&candidate) {
            Ok(()) => return Ok(candidate.canonicalize().unwrap_or(candidate)),
            Err(error) => rejected.push(format!("{} ({error:#})", candidate.display())),
        }
    }

    let rejected = if rejected.is_empty() {
        String::new()
    } else {
        format!("；找到但无法运行：{}", rejected.join("、"))
    };
    Err(anyhow!(
        "未在应用 PATH 或常见安装位置找到可运行的 FFmpeg{rejected}。请手动选择 ffmpeg 可执行文件"
    ))
}

fn ffmpeg_candidates(path: Option<&OsStr>) -> Vec<PathBuf> {
    let executable = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };

    #[allow(unused)]
    let mut candidates = path
        .into_iter()
        .flat_map(env::split_paths)
        .map(|directory| directory.join(executable))
        .collect::<Vec<_>>();

    #[cfg(target_os = "macos")]
    candidates.extend([
        PathBuf::from("/opt/homebrew/bin/ffmpeg"),
        PathBuf::from("/usr/local/bin/ffmpeg"),
        PathBuf::from("/opt/local/bin/ffmpeg"),
    ]);
    #[cfg(all(unix, not(target_os = "macos")))]
    candidates.extend([
        PathBuf::from("/usr/bin/ffmpeg"),
        PathBuf::from("/usr/local/bin/ffmpeg"),
        PathBuf::from("/snap/bin/ffmpeg"),
    ]);

    let mut unique = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if !unique.contains(&candidate) {
            unique.push(candidate);
        }
    }
    unique
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
    rotation: Rotation,
) -> Result<Vec<u8>> {
    validate_range(start, end, cover)?;
    let mut command = FfmpegCommand::new_with_path(ffmpeg);
    command
        .hide_banner()
        .args(["-loglevel", "error", "-ss"])
        .arg(seconds(cover))
        .arg("-i")
        .arg(video);
    if let Some(filter) = rotation.ffmpeg_filter() {
        command.args(["-vf", filter]);
    }
    command.args([
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
    pub rotation: Rotation,
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
        options.rotation,
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
        ]);
    if let Some(filter) = options.rotation.ffmpeg_filter() {
        command.args(["-vf", filter]);
    }
    command
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
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_detection_includes_entries_from_path() {
        let path = env::join_paths([Path::new("/example/one"), Path::new("/example/two")])
            .expect("test PATH should be valid");
        let candidates = ffmpeg_candidates(Some(&path));
        let executable = if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        };
        assert!(candidates.contains(&Path::new("/example/one").join(executable)));
        assert!(candidates.contains(&Path::new("/example/two").join(executable)));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn automatic_detection_includes_macos_package_managers() {
        let candidates = ffmpeg_candidates(Some(OsStr::new("/usr/bin:/bin")));
        assert!(candidates.contains(&PathBuf::from("/opt/homebrew/bin/ffmpeg")));
        assert!(candidates.contains(&PathBuf::from("/usr/local/bin/ffmpeg")));
        assert!(candidates.contains(&PathBuf::from("/opt/local/bin/ffmpeg")));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn automatic_detection_returns_an_absolute_path_for_known_installations() {
        let known_installation = [
            Path::new("/opt/homebrew/bin/ffmpeg"),
            Path::new("/usr/local/bin/ffmpeg"),
            Path::new("/opt/local/bin/ffmpeg"),
        ]
        .into_iter()
        .find(|path| path.is_file());
        let Some(_) = known_installation else {
            return;
        };

        let detected = detect_ffmpeg().expect("a known FFmpeg installation should be detected");
        assert!(detected.is_absolute());
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
            rotation: Rotation::Clockwise90,
            max_output_size: DEFAULT_MAX_OUTPUT_SIZE,
        });
        let _ = fs::remove_file(&output);
        result.unwrap();
    }
}
