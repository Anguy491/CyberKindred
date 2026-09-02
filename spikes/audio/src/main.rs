use std::env;
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufRead, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lofty::file::{AudioFile, TaggedFileExt};
use lofty::tag::Accessor;
use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::{Decoder, DeviceSinkBuilder, Player};
use serde::Serialize;
use sha2::{Digest, Sha256};
use symphonia::core::audio::sample::Sample;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;

const SCHEMA_VERSION: &str = "1.0.0";
const SUPPORTED_FIXTURES: [&str; 6] = [
    "tone.mp3",
    "tone.flac",
    "tone.m4a",
    "tone.aac",
    "tone.wav",
    "tone.ogg",
];
const CORRUPT_FIXTURE: &str = "corrupt.mp3";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DependencyVersions {
    rodio: &'static str,
    direct_symphonia: &'static str,
    lofty: &'static str,
    rodio_transitive_symphonia: &'static str,
}

impl Default for DependencyVersions {
    fn default() -> Self {
        Self {
            rodio: "0.22.2",
            direct_symphonia: "0.6.1",
            lofty: "0.25.1",
            rodio_transitive_symphonia: "0.5.5",
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SafeError {
    class: &'static str,
    message: &'static str,
}

impl SafeError {
    const fn new(class: &'static str, message: &'static str) -> Self {
        Self { class, message }
    }
}

type ProbeResult<T> = Result<T, SafeError>;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DecodeSummary {
    codec: String,
    sample_rate_hz: Option<u32>,
    channels: Option<u32>,
    packets: u64,
    decoded_samples: u64,
    recoverable_packet_errors: u64,
    peak_absolute_sample: f32,
    elapsed_ms: u128,
    media_to_decode_ratio_x100: Option<u128>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MetadataSummary {
    file_type: String,
    duration_ms: u128,
    sample_rate_hz: Option<u32>,
    channels: Option<u8>,
    audio_bitrate_kbps: Option<u32>,
    bit_depth: Option<u8>,
    tag_count: usize,
    title_present: bool,
    artist_present: bool,
    album_present: bool,
    artwork_count: usize,
    elapsed_ms: u128,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InspectionReport {
    schema_version: &'static str,
    command: &'static str,
    fixture_id: String,
    extension: String,
    content_sha256: Option<String>,
    file_bytes: Option<u64>,
    hash_matches_manifest: Option<bool>,
    dependencies: DependencyVersions,
    decode: Option<DecodeSummary>,
    metadata: Option<MetadataSummary>,
    error: Option<SafeError>,
    status: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MatrixReport {
    schema_version: &'static str,
    command: &'static str,
    dependencies: DependencyVersions,
    fixtures: Vec<InspectionReport>,
    supported_formats_passed: usize,
    corrupt_fixture_isolated: bool,
    manifest_complete: bool,
    status: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceSummary {
    index: usize,
    name: String,
    device_id_sha256: String,
    is_default: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceReport {
    schema_version: &'static str,
    command: &'static str,
    devices: Vec<DeviceSummary>,
    status: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlaybackEvent<'a> {
    schema_version: &'static str,
    command: &'static str,
    event: &'a str,
    timestamp_unix_ms: u128,
    position_ms: u128,
    paused: bool,
    device_name: &'a str,
    device_id_sha256: &'a str,
    latency_ms: u128,
    stream_error_count: usize,
    detail: Option<&'a str>,
}

struct PlaybackSession {
    player: Player,
    _device_sink: rodio::MixerDeviceSink,
    device_name: String,
    device_id_sha256: String,
    stream_errors: Arc<Mutex<Vec<&'static str>>>,
}

fn main() -> ExitCode {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let envelope = serde_json::json!({
                "schemaVersion": SCHEMA_VERSION,
                "status": "fail",
                "error": error,
            });
            print_json(&envelope);
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[OsString]) -> ProbeResult<()> {
    let Some(command) = args.first().and_then(|value| value.to_str()) else {
        print_usage();
        return Err(SafeError::new("invalid_input", "缺少探针命令。"));
    };

    match command {
        "devices" if args.len() == 1 => {
            print_json(&list_devices()?);
            Ok(())
        }
        "inspect" if args.len() == 2 => {
            let path = PathBuf::from(&args[1]);
            let report = inspect_path(&path, None);
            let passed = report.status == "pass";
            print_json(&report);
            if passed {
                Ok(())
            } else {
                Err(SafeError::new("inspection_failed", "音频检查未通过。"))
            }
        }
        "matrix" if args.len() <= 2 => {
            let directory = args.get(1).map_or_else(
                || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures"),
                PathBuf::from,
            );
            let report = run_matrix(&directory)?;
            let passed = report.status == "pass";
            print_json(&report);
            if passed {
                Ok(())
            } else {
                Err(SafeError::new(
                    "fixture_matrix_failed",
                    "fixture 矩阵未满足人工验收前置条件。",
                ))
            }
        }
        "play" if args.len() == 2 => run_player(&PathBuf::from(&args[1])),
        _ => {
            print_usage();
            Err(SafeError::new("invalid_input", "命令或参数数量不正确。"))
        }
    }
}

fn print_usage() {
    eprintln!("CyberKindred TASK-002 audio probe");
    eprintln!("  devices");
    eprintln!("  inspect <audio-file>");
    eprintln!("  matrix [fixture-directory]");
    eprintln!("  play <audio-file>");
}

fn print_json(value: &impl Serialize) {
    match serde_json::to_string(value) {
        Ok(json) => println!("{json}"),
        Err(_) => eprintln!("无法序列化脱敏探针结果。"),
    }
}

fn inspect_path(path: &Path, expected_hash: Option<&str>) -> InspectionReport {
    let raw_file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .map_or_else(|| "[NON_UTF8_NAME]".to_owned(), ToOwned::to_owned);
    let fixture_id = if SUPPORTED_FIXTURES.contains(&raw_file_name.as_str())
        || raw_file_name == CORRUPT_FIXTURE
    {
        raw_file_name
    } else {
        "[EXTERNAL_AUDIO]".to_owned()
    };
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map_or_else(|| "unknown".to_owned(), str::to_lowercase);

    let (content_sha256, file_bytes, hash_error) = match hash_file(path) {
        Ok((hash, bytes)) => (Some(hash), Some(bytes), None),
        Err(error) => (None, None, Some(error)),
    };

    let decode_result = if hash_error.is_none() {
        decode_with_symphonia(path, &extension)
    } else {
        Err(SafeError::new("file_open_failed", "无法读取输入音频文件。"))
    };
    let metadata_result = if hash_error.is_none() {
        read_metadata(path)
    } else {
        Err(SafeError::new("file_open_failed", "无法读取输入音频文件。"))
    };

    let hash_matches_manifest = expected_hash.map(|expected| {
        content_sha256
            .as_deref()
            .is_some_and(|actual| actual.eq_ignore_ascii_case(expected))
    });

    let mut error = hash_error;
    let decode = match decode_result {
        Ok(summary) => Some(summary),
        Err(decode_error) => {
            error = Some(decode_error);
            None
        }
    };
    let metadata = match metadata_result {
        Ok(summary) => Some(summary),
        Err(metadata_error) => {
            if error.is_none() {
                error = Some(metadata_error);
            }
            None
        }
    };

    let status = if error.is_none() && hash_matches_manifest != Some(false) {
        "pass"
    } else {
        "fail"
    };

    InspectionReport {
        schema_version: SCHEMA_VERSION,
        command: "inspect",
        fixture_id,
        extension,
        content_sha256,
        file_bytes,
        hash_matches_manifest,
        dependencies: DependencyVersions::default(),
        decode,
        metadata,
        error,
        status,
    }
}

fn hash_file(path: &Path) -> ProbeResult<(String, u64)> {
    let mut file = File::open(path)
        .map_err(|_| SafeError::new("file_open_failed", "无法打开输入音频文件。"))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    let mut total = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| SafeError::new("file_read_failed", "读取输入音频失败。"))?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        hasher.update(&buffer[..read]);
    }
    Ok((hex::encode(hasher.finalize()), total))
}

// Keeping the probe, decoder construction, and packet loop linear makes each failure stage auditable.
#[allow(clippy::too_many_lines)]
fn decode_with_symphonia(path: &Path, extension: &str) -> ProbeResult<DecodeSummary> {
    let started = Instant::now();
    let file = File::open(path)
        .map_err(|_| SafeError::new("file_open_failed", "无法打开输入音频文件。"))?;
    let media_source = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());
    let mut hint = Hint::new();
    if extension != "unknown" {
        hint.with_extension(extension);
    }

    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            media_source,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|_| {
            SafeError::new(
                "unsupported_or_corrupt_format",
                "Symphonia 无法识别或打开该音频容器。",
            )
        })?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| SafeError::new("no_audio_track", "容器中没有可解码音轨。"))?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .ok_or_else(|| SafeError::new("missing_codec_parameters", "音轨缺少 codec 参数。"))?;
    let audio_params = params
        .audio()
        .ok_or_else(|| SafeError::new("not_audio_codec", "默认音轨不是音频 codec。"))?;
    let codec = format!("{:?}", audio_params.codec);
    let sample_rate_hz = audio_params.sample_rate;
    let channels = audio_params
        .channels
        .as_ref()
        .and_then(|channels| u32::try_from(channels.count()).ok());
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(audio_params, &AudioDecoderOptions::default())
        .map_err(|_| {
            SafeError::new(
                "unsupported_codec",
                "Symphonia 未启用或不支持该音频 codec。",
            )
        })?;

    let mut packets = 0_u64;
    let mut decoded_samples = 0_u64;
    let mut recoverable_packet_errors = 0_u64;
    let mut peak_absolute_sample = 0.0_f32;
    let mut scratch = Vec::<f32>::new();

    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(SymphoniaError::ResetRequired) => {
                return Err(SafeError::new(
                    "decoder_reset_required",
                    "音频流在解码期间改变了轨道布局。",
                ));
            }
            Err(_) => {
                return Err(SafeError::new(
                    "packet_read_failed",
                    "读取音频 packet 失败。",
                ));
            }
        };

        if packet.track_id != track_id {
            continue;
        }
        packets = packets.saturating_add(1);

        match decoder.decode(&packet) {
            Ok(audio_buffer) => {
                let sample_count = audio_buffer.samples_interleaved();
                scratch.resize(sample_count, f32::MID);
                audio_buffer.copy_to_slice_interleaved(&mut scratch);
                decoded_samples = decoded_samples.saturating_add(sample_count as u64);
                for sample in &scratch {
                    peak_absolute_sample = peak_absolute_sample.max(sample.abs());
                }
            }
            Err(SymphoniaError::DecodeError(_) | SymphoniaError::IoError(_)) => {
                recoverable_packet_errors = recoverable_packet_errors.saturating_add(1);
            }
            Err(_) => {
                return Err(SafeError::new(
                    "decode_failed",
                    "Symphonia 遇到不可恢复的解码错误。",
                ));
            }
        }
    }

    if decoded_samples == 0 {
        return Err(SafeError::new(
            "decoded_no_samples",
            "音频未产生任何解码样本。",
        ));
    }

    let elapsed_ms = started.elapsed().as_millis();
    let media_to_decode_ratio_x100 = match (sample_rate_hz, channels) {
        (Some(rate), Some(channel_count)) if elapsed_ms > 0 && channel_count > 0 => {
            let denominator = u128::from(rate) * u128::from(channel_count);
            let media_ms = u128::from(decoded_samples).saturating_mul(1000) / denominator;
            Some(media_ms.saturating_mul(100) / elapsed_ms)
        }
        _ => None,
    };

    Ok(DecodeSummary {
        codec,
        sample_rate_hz,
        channels,
        packets,
        decoded_samples,
        recoverable_packet_errors,
        peak_absolute_sample,
        elapsed_ms,
        media_to_decode_ratio_x100,
    })
}

fn read_metadata(path: &Path) -> ProbeResult<MetadataSummary> {
    let started = Instant::now();
    let tagged_file = lofty::read_from_path(path).map_err(|_| {
        SafeError::new("metadata_read_failed", "lofty 无法读取该文件的属性或标签。")
    })?;
    let properties = tagged_file.properties();
    let mut title_present = false;
    let mut artist_present = false;
    let mut album_present = false;
    let mut artwork_count = 0_usize;
    for tag in tagged_file.tags() {
        title_present |= tag.title().is_some();
        artist_present |= tag.artist().is_some();
        album_present |= tag.album().is_some();
        artwork_count = artwork_count.saturating_add(tag.pictures().len());
    }

    Ok(MetadataSummary {
        file_type: format!("{:?}", tagged_file.file_type()),
        duration_ms: properties.duration().as_millis(),
        sample_rate_hz: properties.sample_rate(),
        channels: properties.channels(),
        audio_bitrate_kbps: properties.audio_bitrate(),
        bit_depth: properties.bit_depth(),
        tag_count: tagged_file.tags().len(),
        title_present,
        artist_present,
        album_present,
        artwork_count,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

fn run_matrix(directory: &Path) -> ProbeResult<MatrixReport> {
    let manifest = read_hash_manifest(&directory.join("MANIFEST.sha256"))?;
    let manifest_complete = SUPPORTED_FIXTURES
        .iter()
        .chain(std::iter::once(&CORRUPT_FIXTURE))
        .all(|name| manifest.iter().any(|(_, entry)| entry == name));

    let mut fixtures = Vec::with_capacity(SUPPORTED_FIXTURES.len() + 1);
    for fixture in SUPPORTED_FIXTURES {
        let expected = manifest
            .iter()
            .find_map(|(hash, name)| (name == fixture).then_some(hash.as_str()));
        fixtures.push(inspect_path(&directory.join(fixture), expected));
    }

    let corrupt_expected = manifest
        .iter()
        .find_map(|(hash, name)| (name == CORRUPT_FIXTURE).then_some(hash.as_str()));
    let corrupt_report = inspect_path(&directory.join(CORRUPT_FIXTURE), corrupt_expected);
    let corrupt_fixture_isolated = corrupt_report.status == "fail"
        && corrupt_report.content_sha256.is_some()
        && corrupt_report.hash_matches_manifest == Some(true);
    fixtures.push(corrupt_report);

    let supported_formats_passed = fixtures
        .iter()
        .take(SUPPORTED_FIXTURES.len())
        .filter(|report| report.status == "pass")
        .count();
    let passed = supported_formats_passed == SUPPORTED_FIXTURES.len()
        && corrupt_fixture_isolated
        && manifest_complete;

    Ok(MatrixReport {
        schema_version: SCHEMA_VERSION,
        command: "matrix",
        dependencies: DependencyVersions::default(),
        fixtures,
        supported_formats_passed,
        corrupt_fixture_isolated,
        manifest_complete,
        status: if passed { "pass" } else { "fail" },
    })
}

fn read_hash_manifest(path: &Path) -> ProbeResult<Vec<(String, String)>> {
    let file = File::open(path)
        .map_err(|_| SafeError::new("fixture_manifest_missing", "找不到 fixture 哈希清单。"))?;
    let reader = io::BufReader::new(file);
    let mut entries = Vec::new();
    for line in reader.lines() {
        let line = line.map_err(|_| {
            SafeError::new(
                "fixture_manifest_read_failed",
                "读取 fixture 哈希清单失败。",
            )
        })?;
        let Some((hash, name)) = line.split_once("  ") else {
            return Err(SafeError::new(
                "fixture_manifest_invalid",
                "fixture 哈希清单格式无效。",
            ));
        };
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(SafeError::new(
                "fixture_manifest_invalid",
                "fixture 哈希清单包含无效 SHA-256。",
            ));
        }
        if name.contains(['/', '\\']) || name.is_empty() {
            return Err(SafeError::new(
                "fixture_manifest_invalid",
                "fixture 哈希清单包含无效文件名。",
            ));
        }
        entries.push((hash.to_lowercase(), name.to_owned()));
    }
    Ok(entries)
}

fn list_devices() -> ProbeResult<DeviceReport> {
    let host = rodio::cpal::default_host();
    let default_id = host
        .default_output_device()
        .map(|device| format!("{:?}", device.id()));
    let output_devices = host.output_devices().map_err(|_| {
        SafeError::new(
            "audio_device_enumeration_failed",
            "无法枚举 Windows 音频输出设备。",
        )
    })?;

    let mut devices = Vec::new();
    for (index, device) in output_devices.enumerate() {
        let raw_id = format!("{:?}", device.id());
        let is_default = default_id.as_deref() == Some(raw_id.as_str());
        let name = device.description().map_or_else(
            |_| "[UNKNOWN_DEVICE]".to_owned(),
            |value| value.name().to_owned(),
        );
        devices.push(DeviceSummary {
            index,
            name,
            device_id_sha256: hash_text(&raw_id),
            is_default,
        });
    }

    Ok(DeviceReport {
        schema_version: SCHEMA_VERSION,
        command: "devices",
        status: if devices.is_empty() { "fail" } else { "pass" },
        devices,
    })
}

fn hash_text(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn open_default_session(path: &Path, position: Duration) -> ProbeResult<PlaybackSession> {
    let host = rodio::cpal::default_host();
    let device = host.default_output_device().ok_or_else(|| {
        SafeError::new("audio_output_unavailable", "Windows 没有默认音频输出设备。")
    })?;
    let raw_device_id = format!("{:?}", device.id());
    let device_name = device.description().map_or_else(
        |_| "[UNKNOWN_DEVICE]".to_owned(),
        |value| value.name().to_owned(),
    );
    let stream_errors = Arc::new(Mutex::new(Vec::new()));
    let callback_errors = Arc::clone(&stream_errors);
    let builder = DeviceSinkBuilder::from_device(device)
        .map_err(|_| SafeError::new("audio_output_unavailable", "默认音频输出设备配置不可用。"))?
        .with_error_callback(move |_| {
            if let Ok(mut errors) = callback_errors.lock() {
                errors.push("os_stream_error");
            }
        });
    let device_sink = builder
        .open_sink_or_fallback()
        .map_err(|_| SafeError::new("audio_output_unavailable", "无法打开默认音频输出设备。"))?;
    let player = Player::connect_new(device_sink.mixer());
    player.pause();
    player.set_volume(0.25);

    let file = File::open(path)
        .map_err(|_| SafeError::new("file_open_failed", "无法打开要播放的音频文件。"))?;
    let decoder = Decoder::try_from(file)
        .map_err(|_| SafeError::new("rodio_decode_failed", "rodio 播放解码器无法打开该音频。"))?;
    player.append(decoder);
    if !position.is_zero() {
        player
            .try_seek(position)
            .map_err(|_| SafeError::new("seek_failed", "重建输出时无法恢复保存位置。"))?;
    }

    Ok(PlaybackSession {
        player,
        _device_sink: device_sink,
        device_name,
        device_id_sha256: hash_text(&raw_device_id),
        stream_errors,
    })
}

// This throwaway interactive loop intentionally keeps each manual control adjacent to its evidence event.
#[allow(clippy::too_many_lines)]
fn run_player(path: &Path) -> ProbeResult<()> {
    let inspection = inspect_path(path, None);
    let inspect_passed = inspection.status == "pass";
    print_json(&inspection);
    if !inspect_passed {
        return Err(SafeError::new(
            "preflight_failed",
            "Symphonia/lofty 播放前检查失败。",
        ));
    }

    let started = Instant::now();
    let mut session = open_default_session(path, Duration::ZERO)?;
    emit_playback_event(&session, "loaded_paused", started.elapsed(), None);
    eprintln!(
        "已加载并保持暂停。命令：p 播放/暂停；s <秒> seek；d 切换到当前系统默认设备；l 枚举设备；r 状态；q 退出。"
    );

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line.map_err(|_| SafeError::new("stdin_read_failed", "读取交互命令失败。"))?;
        let mut tokens = line.split_whitespace();
        let Some(command) = tokens.next() else {
            continue;
        };
        match command {
            "p" if tokens.next().is_none() => {
                let action_started = Instant::now();
                let event = if session.player.is_paused() {
                    session.player.play();
                    "play"
                } else {
                    session.player.pause();
                    "pause"
                };
                emit_playback_event(&session, event, action_started.elapsed(), None);
            }
            "s" => {
                let Some(seconds_text) = tokens.next() else {
                    eprintln!("seek 用法：s <非负秒数>");
                    continue;
                };
                if tokens.next().is_some() {
                    eprintln!("seek 只接受一个秒数参数。");
                    continue;
                }
                let Ok(seconds) = seconds_text.parse::<f64>() else {
                    eprintln!("seek 秒数格式无效。");
                    continue;
                };
                if !seconds.is_finite() || seconds.is_sign_negative() {
                    eprintln!("seek 秒数必须是有限非负数。");
                    continue;
                }
                let action_started = Instant::now();
                match session.player.try_seek(Duration::from_secs_f64(seconds)) {
                    Ok(()) => emit_playback_event(
                        &session,
                        "seek",
                        action_started.elapsed(),
                        Some("absolute_seconds"),
                    ),
                    Err(_) => emit_playback_event(
                        &session,
                        "seek_failed",
                        action_started.elapsed(),
                        Some("seek_not_supported_or_decode_failed"),
                    ),
                }
            }
            "d" if tokens.next().is_none() => {
                session.player.pause();
                let saved_position = session.player.get_pos();
                let prior_device_id = session.device_id_sha256.clone();
                let action_started = Instant::now();
                match open_default_session(path, saved_position) {
                    Ok(new_session) => {
                        session = new_session;
                        let detail = if session.device_id_sha256 == prior_device_id {
                            "same_default_device_reopened_paused"
                        } else {
                            "new_default_device_opened_paused"
                        };
                        emit_playback_event(
                            &session,
                            "default_device_switched",
                            action_started.elapsed(),
                            Some(detail),
                        );
                        eprintln!("输出已重建并保持暂停；确认设备后按 p 才会继续出声。");
                    }
                    Err(error) => {
                        emit_playback_event(
                            &session,
                            "default_device_switch_failed",
                            action_started.elapsed(),
                            Some(error.class),
                        );
                        eprintln!("切换失败；旧播放器保持暂停。请修复默认输出后再次输入 d。");
                    }
                }
            }
            "l" if tokens.next().is_none() => print_json(&list_devices()?),
            "r" if tokens.next().is_none() => {
                emit_playback_event(&session, "status", Duration::ZERO, None);
            }
            "q" if tokens.next().is_none() => {
                let action_started = Instant::now();
                session.player.stop();
                emit_playback_event(&session, "stopped", action_started.elapsed(), None);
                return Ok(());
            }
            _ => eprintln!("未知命令。可用：p；s <秒>；d；l；r；q。"),
        }
    }

    session.player.stop();
    emit_playback_event(&session, "stdin_closed_stopped", Duration::ZERO, None);
    Ok(())
}

fn emit_playback_event(
    session: &PlaybackSession,
    event: &str,
    latency: Duration,
    detail: Option<&str>,
) {
    let stream_error_count = session
        .stream_errors
        .lock()
        .map_or(0, |errors| errors.len());
    let report = PlaybackEvent {
        schema_version: SCHEMA_VERSION,
        command: "play",
        event,
        timestamp_unix_ms: unix_timestamp_ms(),
        position_ms: session.player.get_pos().as_millis(),
        paused: session.player.is_paused(),
        device_name: &session.device_name,
        device_id_sha256: &session.device_id_sha256,
        latency_ms: latency.as_millis(),
        stream_error_count,
        detail,
    };
    print_json(&report);
}

fn unix_timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}
