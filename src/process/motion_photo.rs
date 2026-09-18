//! Google Motion Photo 导出。
//!
//! 这里不依赖 FFmpeg 或平台媒体框架：`mp4` 负责读取和重封装 ISO MP4，
//! `openh264` 与 `rust_h265` 分别负责把 AVC / HEVC 的指定帧解成封面 JPEG。
//! 输出不复制音轨，以保持这条纯 Rust 管线小而确定。

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
use rust_h265::{Decoder as HevcDecoder, Frame as HevcFrame, parse_hvcc};

const MAX_DURATION: Duration = Duration::from_secs(10);

/// 供界面展示的输入视频信息。
#[derive(Debug, Clone, Copy)]
pub struct MotionPhotoVideoInfo {
    pub duration: Duration,
    pub width: u16,
    pub height: u16,
}

/// 读取并验证可用于 Motion Photo 的 MP4 输入。
pub fn inspect_motion_photo_video(video_path: &Path) -> Result<MotionPhotoVideoInfo> {
    let source = File::open(video_path)?;
    let source_size = source.metadata()?.len();
    let reader = Mp4Reader::read_header(std::io::BufReader::new(source), source_size)?;
    let video_track = find_video_track(&reader, video_path)?;
    let track = reader.tracks().get(&video_track).unwrap();
    Ok(MotionPhotoVideoInfo {
        duration: reader.duration(),
        width: track.width(),
        height: track.height(),
    })
}

/// 解出指定时间点的 JPEG 封面；用于界面预览，也和最终导出共用同一解码路径。
pub fn render_motion_photo_cover(
    video_path: &Path,
    start: Duration,
    end: Duration,
    cover_time: Duration,
) -> Result<Vec<u8>> {
    ensure!(start < end, "结束时间必须晚于起始时间");
    ensure!(
        end - start <= MAX_DURATION,
        "Motion Photo 视频最长只能为 10 秒"
    );
    ensure!(
        (start..end).contains(&cover_time),
        "封面帧必须落在所选的视频区间内"
    );
    let source = File::open(video_path)?;
    let size = source.metadata()?.len();
    let mut reader = Mp4Reader::read_header(std::io::BufReader::new(source), size)?;
    let id = find_video_track(&reader, video_path)?;
    let codec = codec_for_track(&reader, id, video_path)?;
    let (scale, width, height, h264_config) = {
        let track = reader.tracks().get(&id).expect("视频轨道来自同一个 reader");
        let h264_config = if matches!(codec, MediaType::H264) {
            let avcc = &track.trak.mdia.minf.stbl.stsd.avc1.as_ref().unwrap().avcc;
            Some((
                track.sequence_parameter_set()?.to_vec(),
                track.picture_parameter_set()?.to_vec(),
                avcc.length_size_minus_one as usize + 1,
            ))
        } else {
            None
        };
        (
            track.timescale(),
            track.width(),
            track.height(),
            h264_config,
        )
    };
    let samples = selected_samples(&mut reader, id, scale, start, end)?;
    ensure!(!samples.is_empty(), "所选时间范围内没有可用的视频帧");
    match codec {
        MediaType::H264 => {
            let (sps, pps, nal_length_size) = h264_config.expect("H.264 轨道应包含 avcC");
            decode_h264_cover(
                &samples,
                &sps,
                &pps,
                nal_length_size,
                scale,
                cover_time,
                width,
                height,
                88,
            )
        }
        MediaType::H265 => decode_hevc_cover(video_path, &samples, scale, cover_time, 88),
        _ => unreachable!("已由 find_supported_video_track 验证"),
    }
}

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
    let video_track = find_video_track(&reader, options.video_path)?;
    let codec = codec_for_track(&reader, video_track, options.video_path)?;

    let timescale = reader
        .tracks()
        .get(&video_track)
        .expect("视频轨道来自同一个 reader")
        .timescale();
    let (sps_pps, width, height) = {
        let track = reader
            .tracks()
            .get(&video_track)
            .expect("视频轨道来自同一个 reader");
        let sps_pps = if matches!(codec, MediaType::H264) {
            Some((
                track.sequence_parameter_set()?.to_vec(),
                track.picture_parameter_set()?.to_vec(),
            ))
        } else {
            None
        };
        (sps_pps, track.width(), track.height())
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
    let cover = match codec {
        MediaType::H264 => {
            let (sps, pps) = sps_pps.as_ref().expect("H.264 轨道应包含 avcC");
            let nal_length_size = reader
                .tracks()
                .get(&video_track)
                .unwrap()
                .trak
                .mdia
                .minf
                .stbl
                .stsd
                .avc1
                .as_ref()
                .unwrap()
                .avcc
                .length_size_minus_one as usize
                + 1;
            decode_h264_cover(
                &samples,
                &sps,
                &pps,
                nal_length_size,
                timescale,
                options.cover_time,
                width,
                height,
                options.jpeg_quality,
            )?
        }
        MediaType::H265 => decode_hevc_cover(
            options.video_path,
            &samples,
            timescale,
            options.cover_time,
            options.jpeg_quality,
        )?,
        _ => unreachable!("已由 find_supported_video_track 验证"),
    };
    // 不裁切时保留相机原始 MP4。除了避免无谓的复制，这对手机图库尤其重要：
    // 原视频的色彩描述、edit list、音轨及厂商私有 box 都不会在重封装时丢失。
    let video = if options.start.is_zero() && options.end == source_duration {
        fs::read(options.video_path)
            .with_context(|| format!("无法读取视频：{}", options.video_path.display()))?
    } else {
        ensure!(
            matches!(codec, MediaType::H264),
            "HEVC/H.265 暂不支持裁切重封装；请选择完整视频后导出"
        );
        let (sps, pps) = sps_pps.as_ref().expect("H.264 轨道应包含 avcC");
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

fn find_supported_video_track(reader: &Mp4Reader<std::io::BufReader<File>>) -> Result<u32> {
    reader
        .tracks()
        .values()
        .find(|track| {
            matches!(track.track_type(), Ok(TrackType::Video))
                && matches!(track.media_type(), Ok(MediaType::H264 | MediaType::H265))
        })
        .map(|track| track.track_id())
        .ok_or_else(|| anyhow!("只支持包含 H.264/AVC 或 HEVC/H.265 视频轨的 MP4 文件"))
}

/// `mp4` 0.14 尚未把常见的 `hvc1` sample entry 映射为 `MediaType::H265`，
/// 但它仍能读取同一轨道的 sample table，因此在此做一个受限容器回退。
fn find_video_track(
    reader: &Mp4Reader<std::io::BufReader<File>>,
    video_path: &Path,
) -> Result<u32> {
    find_supported_video_track(reader).or_else(|_| {
        if is_hvc1_mp4(video_path)? {
            reader
                .tracks()
                .values()
                .find(|track| matches!(track.track_type(), Ok(TrackType::Video)))
                .map(|track| track.track_id())
                .ok_or_else(|| anyhow!("MP4 不含视频轨"))
        } else {
            Err(anyhow!(
                "只支持包含 H.264/AVC 或 HEVC/H.265 视频轨的 MP4 文件"
            ))
        }
    })
}

fn codec_for_track(
    reader: &Mp4Reader<std::io::BufReader<File>>,
    track_id: u32,
    _video_path: &Path,
) -> Result<MediaType> {
    match reader.tracks().get(&track_id).unwrap().media_type() {
        Ok(MediaType::H264) => Ok(MediaType::H264),
        Ok(MediaType::H265) => Ok(MediaType::H265),
        // `find_video_track` 仅会为已确认 `hvc1` 的轨道走到这里。
        _ => Ok(MediaType::H265),
    }
}

fn is_hvc1_mp4(video_path: &Path) -> Result<bool> {
    Ok(fs::read(video_path)
        .with_context(|| format!("无法读取视频：{}", video_path.display()))?
        .windows(4)
        .any(|window| window == b"hvc1"))
}

fn decode_h264_cover(
    samples: &[Mp4Sample],
    sps: &[u8],
    pps: &[u8],
    nal_length_size: usize,
    timescale: u32,
    cover_time: Duration,
    _width: u16,
    _height: u16,
    quality: u8,
) -> Result<Vec<u8>> {
    let mut decoder = Decoder::new().context("无法初始化 H.264 解码器")?;
    let mut parameter_sets = Vec::with_capacity(sps.len() + pps.len() + 8);
    append_annex_b_nal(&mut parameter_sets, sps);
    append_annex_b_nal(&mut parameter_sets, pps);
    let _ = decoder
        .decode(&parameter_sets)
        .context("无法读取 H.264 参数集")?;

    let requested = to_track_time(cover_time, timescale);
    let mut latest = None;
    for sample in samples {
        let mut annex_b = Vec::with_capacity(sample.bytes.len() + 16);
        avcc_sample_to_annex_b(&sample.bytes, nal_length_size, &mut annex_b)?;
        if let Some(frame) = decoder.decode(&annex_b).context("H.264 封面帧解码失败")? {
            let (width, height) = frame.dimensions();
            let mut rgb = vec![0; frame.rgb8_len()];
            frame.write_rgb8(&mut rgb);
            latest = Some((width, height, rgb));
        }
        if sample.start_time >= requested {
            if let Some((width, height, rgb)) = latest.take() {
                return encode_rgb_jpeg(width as u32, height as u32, rgb, quality);
            }
        }
    }
    let (width, height, rgb) = latest.ok_or_else(|| anyhow!("无法解出所选 H.264 封面帧"))?;
    encode_rgb_jpeg(width as u32, height as u32, rgb, quality)
}

fn decode_hevc_cover(
    video_path: &Path,
    samples: &[Mp4Sample],
    timescale: u32,
    cover_time: Duration,
    quality: u8,
) -> Result<Vec<u8>> {
    let bytes = fs::read(video_path)
        .with_context(|| format!("无法读取 HEVC 视频：{}", video_path.display()))?;
    let (nal_length_size, parameter_sets) = hevc_configuration(&bytes)?;
    let mut decoder = HevcDecoder::new();
    for nal in parameter_sets {
        decode_hevc_packet(&mut decoder, &nal, 4)?;
    }

    let requested = to_track_time(cover_time, timescale);
    let mut latest = None;
    for sample in samples {
        if let Some(frame) = decode_hevc_packet(&mut decoder, &sample.bytes, nal_length_size)? {
            latest = Some(frame);
        }
        if sample.start_time >= requested {
            if let Some(frame) = latest.take() {
                return encode_hevc_frame(frame, quality);
            }
        }
    }
    if let Some(frame) = decoder.flush() {
        latest = Some(frame);
    }
    encode_hevc_frame(
        latest.ok_or_else(|| anyhow!("无法解出所选 HEVC 封面帧"))?,
        quality,
    )
}

fn decode_hevc_packet(
    decoder: &mut HevcDecoder,
    packet: &[u8],
    nal_length_size: u8,
) -> Result<Option<HevcFrame>> {
    let mut result = None;
    for nal in parse_hvcc(packet, nal_length_size) {
        if let Some(frame) = decoder
            .decode_nal(&nal)
            .map_err(|error| anyhow!("HEVC 封面帧解码失败：{error}"))?
        {
            result = Some(frame);
        }
    }
    Ok(result)
}

/// 读取 `hvcC` 中的 VPS/SPS/PPS。mp4 0.14 只保留了 hvcC 的版本字段，
/// 因此这里直接解析这个标准化、长度受检的 ISO box，不依赖任何系统媒体框架。
fn hevc_configuration(file: &[u8]) -> Result<(u8, Vec<Vec<u8>>)> {
    let index = file
        .windows(4)
        .position(|window| window == b"hvcC")
        .ok_or_else(|| anyhow!("MP4 缺少 HEVC hvcC 配置"))?;
    ensure!(index >= 4, "HEVC hvcC box 损坏");
    let size = u32::from_be_bytes(file[index - 4..index].try_into().unwrap()) as usize;
    ensure!(
        size >= 30 && index - 4 + size <= file.len(),
        "HEVC hvcC box 长度损坏"
    );
    let data = &file[index + 4..index - 4 + size];
    ensure!(data.len() >= 23 && data[0] == 1, "不支持的 HEVC hvcC 配置");
    let nal_length_size = (data[21] & 0x03) + 1;
    let arrays = data[22] as usize;
    let mut offset = 23;
    let mut parameter_sets = Vec::new();
    for _ in 0..arrays {
        ensure!(offset + 3 <= data.len(), "HEVC hvcC 参数集损坏");
        let nal_type = data[offset] & 0x3f;
        let count = u16::from_be_bytes(data[offset + 1..offset + 3].try_into().unwrap()) as usize;
        offset += 3;
        for _ in 0..count {
            ensure!(offset + 2 <= data.len(), "HEVC hvcC 参数集损坏");
            let length = u16::from_be_bytes(data[offset..offset + 2].try_into().unwrap()) as usize;
            offset += 2;
            ensure!(offset + length <= data.len(), "HEVC hvcC 参数集长度损坏");
            if matches!(nal_type, 32..=34) {
                let mut packet = Vec::with_capacity(length + 4);
                packet.extend_from_slice(&(length as u32).to_be_bytes());
                packet.extend_from_slice(&data[offset..offset + length]);
                parameter_sets.push(packet);
            }
            offset += length;
        }
    }
    ensure!(
        !parameter_sets.is_empty(),
        "HEVC hvcC 不含 VPS/SPS/PPS 参数集"
    );
    Ok((nal_length_size, parameter_sets))
}

fn encode_hevc_frame(frame: HevcFrame, quality: u8) -> Result<Vec<u8>> {
    let width = frame.width as usize;
    let height = frame.height as usize;
    ensure!(
        width > 0 && height > 0 && width % 2 == 0 && height % 2 == 0,
        "HEVC 帧尺寸无效"
    );
    let y_len = width * height;
    let uv_len = y_len / 4;
    ensure!(
        frame.y.len() == y_len && frame.u.len() == uv_len && frame.v.len() == uv_len,
        "HEVC 帧平面尺寸无效"
    );
    let shift = frame.bit_depth.saturating_sub(8);
    let sample = |plane: &rust_h265::PixelData, index: usize| -> Result<i32> {
        match (plane.as_u8(), plane.as_u16()) {
            (Some(values), None) => Ok(values[index] as i32),
            (None, Some(values)) => Ok((values[index] >> shift) as i32),
            _ => Err(anyhow!("HEVC 像素位深无效")),
        }
    };
    let mut rgb = vec![0; y_len * 3];
    for row in 0..height {
        for column in 0..width {
            let y = sample(&frame.y, row * width + column)? - 16;
            let u = sample(&frame.u, (row / 2) * (width / 2) + column / 2)? - 128;
            let v = sample(&frame.v, (row / 2) * (width / 2) + column / 2)? - 128;
            let destination = (row * width + column) * 3;
            rgb[destination] = ((298 * y + 409 * v + 128) >> 8).clamp(0, 255) as u8;
            rgb[destination + 1] = ((298 * y - 100 * u - 208 * v + 128) >> 8).clamp(0, 255) as u8;
            rgb[destination + 2] = ((298 * y + 516 * u + 128) >> 8).clamp(0, 255) as u8;
        }
    }
    encode_rgb_jpeg(frame.width, frame.height, rgb, quality)
}

fn encode_rgb_jpeg(width: u32, height: u32, rgb: Vec<u8>, quality: u8) -> Result<Vec<u8>> {
    let image =
        RgbImage::from_raw(width, height, rgb).ok_or_else(|| anyhow!("无法构造封面 RGB 图像"))?;
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, quality)
        .encode_image(&image)
        .context("无法编码封面 JPEG")?;
    Ok(jpeg)
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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

    #[test]
    fn reads_hevc_parameter_sets_from_hvcc() {
        let mut payload = vec![1; 23];
        payload[21] = 3; // four-byte MP4 NAL lengths
        payload[22] = 1;
        payload.extend_from_slice(&[0x80 | 32, 0, 1, 0, 2, 0x40, 0x01]);
        let mut file = Vec::new();
        file.extend_from_slice(&((payload.len() + 8) as u32).to_be_bytes());
        file.extend_from_slice(b"hvcC");
        file.extend_from_slice(&payload);
        let (length_size, parameter_sets) = hevc_configuration(&file).unwrap();
        assert_eq!(length_size, 4);
        assert_eq!(parameter_sets, vec![vec![0, 0, 0, 2, 0x40, 0x01]]);
    }

    #[test]
    #[ignore = "requires the developer's local HEVC fixture"]
    fn decodes_local_hevc_motion_photo_cover() {
        let path = Path::new("/Users/cakeal/Movies/20260906广东深圳石岩气象梯度观测塔晚霞延时.mp4");
        let info = inspect_motion_photo_video(path).unwrap();
        let end = info.duration.min(MAX_DURATION);
        let jpeg = render_motion_photo_cover(path, Duration::ZERO, end, Duration::ZERO).unwrap();
        assert!(jpeg.starts_with(&[0xff, 0xd8]));
    }
}
