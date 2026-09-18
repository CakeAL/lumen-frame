//! Google Motion Photo 导出。
//!
//! 这里不依赖 FFmpeg 或平台媒体框架：`mp4` 负责读取和重封装 ISO MP4，
//! `openh264` 负责把 H.264 的指定帧解成封面 JPEG。当前只接受 AVC/H.264
//! 视频；输出不复制音轨，以保持这条纯 Rust 管线小而确定。

use std::{
    fs::{self, File},
    io::{BufWriter, Cursor, Write},
    path::Path,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, ensure};
use image::{RgbImage, codecs::jpeg::JpegEncoder};
use mp4::{
    AvcConfig, MediaConfig, MediaType, Mp4Config, Mp4Reader, Mp4Sample, Mp4Writer, TrackConfig,
    TrackType,
};
use openh264::{decoder::Decoder, formats::YUVSource};

const MAX_DURATION: Duration = Duration::from_secs(10);

/// 从输入视频裁出一段并导出为单文件 JPEG Motion Photo。
#[derive(Debug, Clone)]
pub struct MotionPhotoOptions<'a> {
    pub video_path: &'a Path,
    pub output_path: &'a Path,
    /// 裁切起点；输出会从此刻之后的第一个关键帧开始，保证重封装视频可独立解码。
    pub start: Duration,
    /// 裁切终点（不包含）。`end - start` 不得超过 10 秒。
    pub end: Duration,
    /// 用于 JPEG 封面的时间点，必须落在所选区间内。
    pub cover_time: Duration,
    pub jpeg_quality: u8,
}

impl<'a> MotionPhotoOptions<'a> {
    pub fn new(video_path: &'a Path, output_path: &'a Path) -> Self {
        Self {
            video_path,
            output_path,
            start: Duration::ZERO,
            end: MAX_DURATION,
            cover_time: Duration::ZERO,
            jpeg_quality: 92,
        }
    }
}

/// 导出可被 Google Photos / Samsung Gallery 识别的 Motion Photo。
pub fn export_motion_photo(options: &MotionPhotoOptions<'_>) -> Result<()> {
    ensure!(
        options.start < options.end,
        "Motion Photo 的结束时间必须晚于起始时间"
    );
    ensure!(
        options.end - options.start <= MAX_DURATION,
        "Motion Photo 视频最长只能为 10 秒"
    );
    ensure!(
        (options.start..options.end).contains(&options.cover_time),
        "封面帧必须落在所选的视频区间内"
    );
    ensure!(
        (1..=100).contains(&options.jpeg_quality),
        "JPEG 质量必须在 1 到 100 之间"
    );

    let source = File::open(options.video_path)
        .with_context(|| format!("无法打开视频：{}", options.video_path.display()))?;
    let source_size = source.metadata()?.len();
    let mut reader = Mp4Reader::read_header(std::io::BufReader::new(source), source_size)
        .context("无法解析 MP4 容器")?;
    let video_track = find_h264_video_track(&reader)?;

    let timescale = reader
        .tracks()
        .get(&video_track)
        .expect("视频轨道来自同一个 reader")
        .timescale();
    let (sps, pps, nal_length_size, width, height) = {
        let track = reader
            .tracks()
            .get(&video_track)
            .expect("视频轨道来自同一个 reader");
        let avcc = &track.trak.mdia.minf.stbl.stsd.avc1.as_ref().unwrap().avcc;
        (
            track.sequence_parameter_set()?.to_vec(),
            track.picture_parameter_set()?.to_vec(),
            avcc.length_size_minus_one as usize + 1,
            track.width(),
            track.height(),
        )
    };
    let source_duration = reader.duration();
    ensure!(options.end <= source_duration, "所选结束时间超出视频时长");

    let samples = selected_samples(
        &mut reader,
        video_track,
        timescale,
        options.start,
        options.end,
    )?;
    ensure!(!samples.is_empty(), "所选时间范围内没有可用的视频帧");
    let first_time = samples[0].start_time;
    let cover = decode_cover(
        &samples,
        &sps,
        &pps,
        nal_length_size,
        timescale,
        options.cover_time,
        width,
        height,
        options.jpeg_quality,
    )?;
    // 不裁切时保留相机原始 MP4。除了避免无谓的复制，这对手机图库尤其重要：
    // 原视频的色彩描述、edit list、音轨及厂商私有 box 都不会在重封装时丢失。
    let video = if options.start.is_zero() && options.end == source_duration {
        fs::read(options.video_path)
            .with_context(|| format!("无法读取视频：{}", options.video_path.display()))?
    } else {
        write_trimmed_mp4(
            &reader, timescale, width, height, &sps, &pps, first_time, samples,
        )?
    };
    // HyperOS 要求这里是 JPEG 封面所对应视频帧的真实 PTS；设成 0 只适用于
    // 封面恰好是首帧，否则相册可能无法播放或编辑该动态照片。
    let video_start = Duration::from_secs_f64(first_time as f64 / timescale as f64);
    let presentation_timestamp_us = options
        .cover_time
        .checked_sub(video_start)
        .unwrap_or(Duration::ZERO)
        .as_micros()
        .try_into()
        .context("封面时间戳超出 Motion Photo 元数据范围")?;
    let jpeg = insert_motion_photo_xmp(cover, video.len(), presentation_timestamp_us)?;

    let output = File::create(options.output_path)
        .with_context(|| format!("无法创建输出文件：{}", options.output_path.display()))?;
    let mut output = BufWriter::new(output);
    output.write_all(&jpeg)?;
    output.write_all(&video)?;
    output.flush()?;
    Ok(())
}

fn find_h264_video_track(reader: &Mp4Reader<std::io::BufReader<File>>) -> Result<u32> {
    reader
        .tracks()
        .values()
        .find(|track| {
            matches!(track.track_type(), Ok(TrackType::Video))
                && matches!(track.media_type(), Ok(MediaType::H264))
        })
        .map(|track| track.track_id())
        .ok_or_else(|| anyhow!("只支持包含 H.264/AVC 视频轨的 MP4 文件"))
}

fn selected_samples(
    reader: &mut Mp4Reader<std::io::BufReader<File>>,
    track_id: u32,
    timescale: u32,
    start: Duration,
    end: Duration,
) -> Result<Vec<Mp4Sample>> {
    let start = to_track_time(start, timescale);
    let end = to_track_time(end, timescale);
    let count = reader.sample_count(track_id)?;
    let mut started = false;
    let mut samples = Vec::new();

    for sample_id in 1..=count {
        let Some(sample) = reader.read_sample(track_id, sample_id)? else {
            break;
        };
        if !started {
            if sample.start_time < start || !sample.is_sync {
                continue;
            }
            started = true;
        }
        if sample.start_time >= end {
            break;
        }
        samples.push(sample);
    }
    Ok(samples)
}

#[allow(clippy::too_many_arguments)]
fn decode_cover(
    samples: &[Mp4Sample],
    sps: &[u8],
    pps: &[u8],
    nal_length_size: usize,
    timescale: u32,
    requested_time: Duration,
    width: u16,
    height: u16,
    quality: u8,
) -> Result<Vec<u8>> {
    let requested_time = to_track_time(requested_time, timescale);
    let mut decoder = Decoder::new().context("无法初始化 H.264 解码器")?;
    let mut latest = None;

    for (index, sample) in samples.iter().enumerate() {
        let mut bitstream = Vec::new();
        if index == 0 {
            append_annex_b_nal(&mut bitstream, sps);
            append_annex_b_nal(&mut bitstream, pps);
        }
        avcc_sample_to_annex_b(&sample.bytes, nal_length_size, &mut bitstream)?;
        // OpenH264 对个别带 B 帧或恢复点的 access unit 可能报错；其文档建议继续
        // 喂后续 NAL，让解码器在下一个可恢复帧重新同步，而不是中断整段导出。
        if let Ok(Some(frame)) = decoder.decode(&bitstream) {
            let (frame_width, frame_height) = frame.dimensions();
            let mut rgb = vec![0; frame.rgb8_len()];
            frame.write_rgb8(&mut rgb);
            latest = Some((frame_width, frame_height, rgb));
        }
        if sample.start_time >= requested_time && latest.is_some() {
            break;
        }
    }

    let (frame_width, frame_height, rgb) = latest.ok_or_else(|| anyhow!("无法解出所选封面帧"))?;
    ensure!(
        frame_width == width as usize && frame_height == height as usize,
        "解码出的封面尺寸与 MP4 轨道不一致"
    );
    let image = RgbImage::from_raw(width.into(), height.into(), rgb)
        .ok_or_else(|| anyhow!("无法构造封面 RGB 图像"))?;
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, quality)
        .encode_image(&image)
        .context("无法编码封面 JPEG")?;
    Ok(jpeg)
}

fn write_trimmed_mp4(
    reader: &Mp4Reader<std::io::BufReader<File>>,
    timescale: u32,
    width: u16,
    height: u16,
    sps: &[u8],
    pps: &[u8],
    first_time: u64,
    samples: Vec<Mp4Sample>,
) -> Result<Vec<u8>> {
    let config = Mp4Config {
        major_brand: *reader.major_brand(),
        minor_version: reader.minor_version(),
        compatible_brands: reader.compatible_brands().to_vec(),
        timescale,
    };
    let track = TrackConfig {
        track_type: TrackType::Video,
        timescale,
        language: "und".into(),
        media_conf: MediaConfig::AvcConfig(AvcConfig {
            width,
            height,
            seq_param_set: sps.to_vec(),
            pic_param_set: pps.to_vec(),
        }),
    };
    let mut writer = Mp4Writer::write_start(Cursor::new(Vec::new()), &config)?;
    writer.add_track(&track)?;
    // `mp4` 0.14 生成的 ctts 是 version 0；该版本只允许非负 composition
    // offset。源视频有 B 帧时常会有负 offset，直接复制会生成部分手机无法播放的
    // 非法 MP4。整体平移所有 offset 不改变帧之间的显示顺序，只会产生极短的首帧延迟。
    let offset_shift = samples
        .iter()
        .map(|sample| sample.rendering_offset)
        .min()
        .unwrap_or(0)
        .min(0);
    for mut sample in samples {
        sample.start_time -= first_time;
        sample.rendering_offset -= offset_shift;
        writer.write_sample(1, &sample)?;
    }
    writer.write_end()?;
    Ok(writer.into_writer().into_inner())
}

fn insert_motion_photo_xmp(
    mut jpeg: Vec<u8>,
    video_size: usize,
    presentation_timestamp_us: u64,
) -> Result<Vec<u8>> {
    ensure!(jpeg.starts_with(&[0xff, 0xd8]), "封面不是有效 JPEG");
    // `nom-exif`（也有不少图库）会先寻找 TIFF EXIF APP1；纯编码器输出的
    // JPEG 没有这一段时，它们可能不会继续扫描后面的 XMP。写入空 IFD0 即可，
    // 不伪造相机、时间等 EXIF 信息。
    const EMPTY_EXIF: &[u8] = b"Exif\0\0MM\0*\0\0\0\x08\0\0\0\0\0\0\0\0";
    let xmp = format!(
        concat!(
            "http://ns.adobe.com/xap/1.0/\0",
            "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"Adobe XMP Core 5.1.0-jc003\">\n",
            "  <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n",
            "    <rdf:Description\n",
            "      xmlns:GCamera=\"http://ns.google.com/photos/1.0/camera/\"\n",
            "      xmlns:Container=\"http://ns.google.com/photos/1.0/container/\"\n",
            "      xmlns:Item=\"http://ns.google.com/photos/1.0/container/item/\"\n",
            "      GCamera:MotionPhoto=\"1\"\n",
            "      GCamera:MotionPhotoVersion=\"1\"\n",
            "      GCamera:MotionPhotoPresentationTimestampUs=\"{}\">\n",
            "      <Container:Directory>\n",
            "        <rdf:Seq>\n",
            "          <rdf:li rdf:parseType=\"Resource\">\n",
            "            <Container:Item\n",
            "              Item:Mime=\"image/jpeg\"\n",
            "              Item:Semantic=\"Primary\"/>\n",
            "          </rdf:li>\n",
            "          <rdf:li rdf:parseType=\"Resource\">\n",
            "            <Container:Item\n",
            "              Item:Mime=\"video/mp4\"\n",
            "              Item:Semantic=\"MotionPhoto\"\n",
            "              Item:Length=\"{}\"\n",
            "              Item:Padding=\"0\"/>\n",
            "          </rdf:li>\n",
            "        </rdf:Seq>\n",
            "      </Container:Directory>\n",
            "    </rdf:Description>\n",
            "  </rdf:RDF>\n",
            "</x:xmpmeta>"
        ),
        presentation_timestamp_us, video_size
    );
    ensure!(xmp.len() + 2 <= u16::MAX as usize, "Motion Photo XMP 过长");
    // `nom-exif` 的 JPEG EXIF 入口要求在 SOS 前能找到有效 EXIF APP1；空 IFD0
    // 放在前面既不伪造拍摄信息，也让随后的标准 XMP 能被图库扫描。
    let mut segments = app1_segment(EMPTY_EXIF)?;
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

fn avcc_sample_to_annex_b(
    sample: &[u8],
    nal_length_size: usize,
    output: &mut Vec<u8>,
) -> Result<()> {
    ensure!((1..=4).contains(&nal_length_size), "不支持的 AVC NAL 长度");
    let mut offset = 0;
    while offset < sample.len() {
        ensure!(
            offset + nal_length_size <= sample.len(),
            "H.264 样本的 NAL 长度损坏"
        );
        let mut nal_size = 0usize;
        for byte in &sample[offset..offset + nal_length_size] {
            nal_size = (nal_size << 8) | *byte as usize;
        }
        offset += nal_length_size;
        ensure!(
            offset + nal_size <= sample.len(),
            "H.264 样本的 NAL 数据损坏"
        );
        append_annex_b_nal(output, &sample[offset..offset + nal_size]);
        offset += nal_size;
    }
    Ok(())
}

fn append_annex_b_nal(output: &mut Vec<u8>, nal: &[u8]) {
    output.extend_from_slice(&[0, 0, 0, 1]);
    output.extend_from_slice(nal);
}

fn to_track_time(duration: Duration, timescale: u32) -> u64 {
    duration.as_secs() * timescale as u64
        + duration.subsec_nanos() as u64 * timescale as u64 / 1_000_000_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_an_xmp_segment_after_jpeg_soi() {
        let jpeg = insert_motion_photo_xmp(vec![0xff, 0xd8, 0xff, 0xd9], 123, 456).unwrap();
        assert!(
            jpeg.windows(b"GCamera:MotionPhoto=\"1\"".len())
                .any(|x| x == b"GCamera:MotionPhoto=\"1\"")
        );
        assert_eq!(&jpeg[..2], &[0xff, 0xd8]);
    }

    #[test]
    fn converts_avcc_nals_to_annex_b() {
        let mut output = Vec::new();
        avcc_sample_to_annex_b(&[0, 0, 0, 2, 0x65, 0x88], 4, &mut output).unwrap();
        assert_eq!(output, [0, 0, 0, 1, 0x65, 0x88]);
    }
}
