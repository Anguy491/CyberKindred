use std::{
    fs::File,
    io,
    num::{NonZeroU16, NonZeroU32},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use rodio::{DeviceSinkBuilder, MixerDeviceSink, Player, Source};
use symphonia::core::{
    codecs::audio::{AudioDecoder as SymphoniaDecoder, AudioDecoderOptions},
    errors::Error as SymphoniaError,
    formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType, probe::Hint},
    io::{MediaSourceStream, MediaSourceStreamOptions},
    meta::MetadataOptions,
    units::Time,
};
use uuid::Uuid;

use super::engine::{
    AudioEngineError, AudioSnapshot, LocalAudioEngine, LocalAudioEngineEvent,
    LocalAudioEngineEventSink, LocalTrack,
};

const DEFAULT_VOLUME: f32 = 0.7;
const MAX_PACKET_SECONDS: usize = 10;
const MAX_CONSECUTIVE_DECODE_ERRORS: usize = 32;

/// Production output adapter. Construction is silent and opens no device;
/// `load(..., autoplay=false)` opens a paused player before appending samples.
pub struct RodioAudioEngine {
    session: Option<RodioSession>,
    signals: Arc<EngineSignals>,
}

impl RodioAudioEngine {
    #[must_use]
    pub fn new() -> Self {
        Self {
            session: None,
            signals: Arc::new(EngineSignals::default()),
        }
    }

    fn session(&self, session_id: Uuid) -> Result<&RodioSession, AudioEngineError> {
        self.session
            .as_ref()
            .filter(|session| session.session_id == session_id)
            .ok_or(AudioEngineError::MediaInvalid)
    }

    fn snapshot_for(session: &RodioSession) -> AudioSnapshot {
        let position_ms = duration_ms(session.player.get_pos()).min(session.duration_ms);
        AudioSnapshot {
            position_ms,
            duration_ms: session.duration_ms,
        }
    }
}

impl Default for RodioAudioEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalAudioEngine for RodioAudioEngine {
    fn set_event_sink(&mut self, sink: Arc<dyn LocalAudioEngineEventSink>) {
        if let Ok(mut slot) = self.signals.event_sink.lock() {
            *slot = Some(sink);
        }
    }

    fn load(
        &mut self,
        track: &LocalTrack,
        session_id: Uuid,
        position_ms: u64,
        autoplay: bool,
    ) -> Result<AudioSnapshot, AudioEngineError> {
        self.stop();
        let source = Symphonia06Source::open(track, session_id, Arc::clone(&self.signals))?;
        let duration_ms = source.duration_ms;
        if position_ms > duration_ms {
            return Err(AudioEngineError::MediaInvalid);
        }
        self.signals.begin_session(session_id, position_ms);
        let callback_signals = Arc::clone(&self.signals);
        let builder = DeviceSinkBuilder::from_default_device()
            .map_err(|_| AudioEngineError::OutputUnavailable)?
            .with_error_callback(move |_| callback_signals.output_lost());
        let device_sink = builder
            .open_sink_or_fallback()
            .map_err(|_| AudioEngineError::OutputUnavailable)?;
        let player = Player::connect_new(device_sink.mixer());
        player.pause();
        player.set_volume(DEFAULT_VOLUME);
        player.append(source);
        if position_ms > 0 {
            player
                .try_seek(Duration::from_millis(position_ms))
                .map_err(|_| AudioEngineError::MediaInvalid)?;
        }
        if autoplay {
            player.play();
        }
        let session = RodioSession {
            session_id,
            player,
            _device_sink: device_sink,
            duration_ms,
        };
        let snapshot = Self::snapshot_for(&session);
        self.session = Some(session);
        Ok(snapshot)
    }

    fn play(&mut self, session_id: Uuid) -> Result<AudioSnapshot, AudioEngineError> {
        let session = self.session(session_id)?;
        session.player.play();
        Ok(Self::snapshot_for(session))
    }

    fn pause(&mut self, session_id: Uuid) -> Result<AudioSnapshot, AudioEngineError> {
        let session = self.session(session_id)?;
        session.player.pause();
        Ok(Self::snapshot_for(session))
    }

    fn seek(
        &mut self,
        session_id: Uuid,
        position_ms: u64,
    ) -> Result<AudioSnapshot, AudioEngineError> {
        let session = self.session(session_id)?;
        if position_ms > session.duration_ms {
            return Err(AudioEngineError::MediaInvalid);
        }
        session
            .player
            .try_seek(Duration::from_millis(position_ms))
            .map_err(|_| AudioEngineError::MediaInvalid)?;
        self.signals
            .position_ms
            .store(position_ms, Ordering::Release);
        Ok(Self::snapshot_for(session))
    }

    fn snapshot(&self, session_id: Uuid) -> Result<AudioSnapshot, AudioEngineError> {
        self.session(session_id).map(Self::snapshot_for)
    }

    fn stop(&mut self) {
        self.signals.clear_session();
        if let Some(session) = self.session.take() {
            session.player.stop();
        }
    }
}

struct RodioSession {
    session_id: Uuid,
    player: Player,
    _device_sink: MixerDeviceSink,
    duration_ms: u64,
}

#[derive(Default)]
struct EngineSignals {
    event_sink: Mutex<Option<Arc<dyn LocalAudioEngineEventSink>>>,
    session_id: Mutex<Option<Uuid>>,
    position_ms: AtomicU64,
    terminal_sent: AtomicBool,
}

impl EngineSignals {
    fn begin_session(&self, session_id: Uuid, position_ms: u64) {
        if let Ok(mut current) = self.session_id.lock() {
            *current = Some(session_id);
        }
        self.position_ms.store(position_ms, Ordering::Release);
        self.terminal_sent.store(false, Ordering::Release);
    }

    fn clear_session(&self) {
        if let Ok(mut current) = self.session_id.lock() {
            *current = None;
        }
        self.terminal_sent.store(true, Ordering::Release);
    }

    fn update_position(&self, session_id: Uuid, position_ms: u64) {
        if self.matches(session_id) {
            self.position_ms.store(position_ms, Ordering::Release);
        }
    }

    fn ended(&self, session_id: Uuid) {
        self.publish_terminal(session_id, |session_id| LocalAudioEngineEvent::Ended {
            session_id,
        });
    }

    fn failed(&self, session_id: Uuid) {
        self.publish_terminal(session_id, |session_id| LocalAudioEngineEvent::Failed {
            session_id,
            error: AudioEngineError::MediaInvalid,
        });
    }

    fn output_lost(&self) {
        let session_id = self.session_id.lock().ok().and_then(|guard| *guard);
        if let Some(session_id) = session_id {
            self.publish(LocalAudioEngineEvent::OutputLost {
                session_id,
                position_ms: self.position_ms.load(Ordering::Acquire),
            });
        }
    }

    fn publish_terminal<F>(&self, session_id: Uuid, event: F)
    where
        F: FnOnce(Uuid) -> LocalAudioEngineEvent,
    {
        if self.matches(session_id) && !self.terminal_sent.swap(true, Ordering::AcqRel) {
            self.publish(event(session_id));
        }
    }

    fn publish(&self, event: LocalAudioEngineEvent) {
        let sink = self.event_sink.lock().ok().and_then(|slot| slot.clone());
        if let Some(sink) = sink {
            sink.publish(event);
        }
    }

    fn matches(&self, session_id: Uuid) -> bool {
        self.session_id
            .lock()
            .is_ok_and(|current| *current == Some(session_id))
    }
}

struct Symphonia06Source {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn SymphoniaDecoder>,
    track_id: u32,
    channels: NonZeroU16,
    sample_rate: NonZeroU32,
    duration_ms: u64,
    samples: Vec<f32>,
    sample_index: usize,
    produced_frames: u64,
    position_base_ms: u64,
    consecutive_decode_errors: usize,
    exhausted: bool,
    session_id: Uuid,
    signals: Arc<EngineSignals>,
}

impl Symphonia06Source {
    fn open(
        track: &LocalTrack,
        session_id: Uuid,
        signals: Arc<EngineSignals>,
    ) -> Result<Self, AudioEngineError> {
        let file =
            File::open(track.canonical_path()).map_err(|_| AudioEngineError::MediaInvalid)?;
        let stream = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());
        let mut hint = Hint::new();
        if let Some(extension) = track
            .canonical_path()
            .extension()
            .and_then(|value| value.to_str())
        {
            hint.with_extension(extension);
        }
        let format = symphonia::default::get_probe()
            .probe(
                &hint,
                stream,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|_| AudioEngineError::MediaInvalid)?;
        Self::from_format(format, track.duration_ms(), session_id, signals)
    }

    fn from_format(
        format: Box<dyn FormatReader>,
        fallback_duration_ms: u64,
        session_id: Uuid,
        signals: Arc<EngineSignals>,
    ) -> Result<Self, AudioEngineError> {
        let track = format
            .default_track(TrackType::Audio)
            .ok_or(AudioEngineError::MediaInvalid)?;
        let track_id = track.id;
        let time_base = track.time_base;
        let track_duration = track.duration;
        let parameters = track
            .codec_params
            .as_ref()
            .ok_or(AudioEngineError::MediaInvalid)?;
        let audio = parameters.audio().ok_or(AudioEngineError::MediaInvalid)?;
        let channels = audio
            .channels
            .as_ref()
            .and_then(|value| u16::try_from(value.count()).ok())
            .and_then(NonZeroU16::new)
            .ok_or(AudioEngineError::MediaInvalid)?;
        let sample_rate = audio
            .sample_rate
            .and_then(NonZeroU32::new)
            .ok_or(AudioEngineError::MediaInvalid)?;
        let duration_ms = time_base
            .zip(track_duration)
            .and_then(|(base, duration)| base.calc_duration(duration))
            .and_then(|duration| u64::try_from(duration.as_millis()).ok())
            .filter(|duration| *duration > 0)
            .unwrap_or(fallback_duration_ms);
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(audio, &AudioDecoderOptions::default())
            .map_err(|_| AudioEngineError::MediaInvalid)?;
        let mut source = Self {
            format,
            decoder,
            track_id,
            channels,
            sample_rate,
            duration_ms,
            samples: Vec::new(),
            sample_index: 0,
            produced_frames: 0,
            position_base_ms: 0,
            consecutive_decode_errors: 0,
            exhausted: false,
            session_id,
            signals,
        };
        if !source.fill_packet() {
            return Err(AudioEngineError::MediaInvalid);
        }
        Ok(source)
    }

    fn fill_packet(&mut self) -> bool {
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => {
                    self.exhausted = true;
                    self.signals.ended(self.session_id);
                    return false;
                }
                Err(_) => {
                    self.exhausted = true;
                    self.signals.failed(self.session_id);
                    return false;
                }
            };
            if packet.track_id != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    let sample_count = decoded.samples_interleaved();
                    let maximum = self.sample_rate.get() as usize
                        * self.channels.get() as usize
                        * MAX_PACKET_SECONDS;
                    if sample_count == 0 {
                        self.consecutive_decode_errors =
                            self.consecutive_decode_errors.saturating_add(1);
                        if self.consecutive_decode_errors > MAX_CONSECUTIVE_DECODE_ERRORS {
                            self.exhausted = true;
                            self.signals.failed(self.session_id);
                            return false;
                        }
                        continue;
                    }
                    if sample_count > maximum {
                        self.exhausted = true;
                        self.signals.failed(self.session_id);
                        return false;
                    }
                    self.samples.resize(sample_count, 0.0);
                    decoded.copy_to_slice_interleaved(&mut self.samples);
                    self.sample_index = 0;
                    self.consecutive_decode_errors = 0;
                    return true;
                }
                Err(SymphoniaError::DecodeError(_) | SymphoniaError::IoError(_)) => {
                    self.consecutive_decode_errors =
                        self.consecutive_decode_errors.saturating_add(1);
                    if self.consecutive_decode_errors > MAX_CONSECUTIVE_DECODE_ERRORS {
                        self.exhausted = true;
                        self.signals.failed(self.session_id);
                        return false;
                    }
                }
                Err(_) => {
                    self.exhausted = true;
                    self.signals.failed(self.session_id);
                    return false;
                }
            }
        }
    }

    fn seek_source(&mut self, position: Duration) -> Result<(), rodio::source::SeekError> {
        let time = Time::try_from_nanos_u128(position.as_nanos()).ok_or_else(seek_error)?;
        self.format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|_| seek_error())?;
        self.decoder.reset();
        self.samples.clear();
        self.sample_index = 0;
        self.produced_frames = 0;
        self.position_base_ms = duration_ms(position).min(self.duration_ms);
        self.consecutive_decode_errors = 0;
        self.exhausted = false;
        self.signals
            .update_position(self.session_id, self.position_base_ms);
        Ok(())
    }
}

impl Iterator for Symphonia06Source {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.sample_index >= self.samples.len() && !self.fill_packet() {
            return None;
        }
        let sample = *self.samples.get(self.sample_index)?;
        self.sample_index = self.sample_index.saturating_add(1);
        if self
            .sample_index
            .is_multiple_of(self.channels.get() as usize)
        {
            self.produced_frames = self.produced_frames.saturating_add(1);
        }
        if self.sample_index == self.samples.len() {
            let decoded_ms =
                self.produced_frames.saturating_mul(1_000) / u64::from(self.sample_rate.get());
            self.signals.update_position(
                self.session_id,
                self.position_base_ms
                    .saturating_add(decoded_ms)
                    .min(self.duration_ms),
            );
        }
        Some(sample)
    }
}

impl Source for Symphonia06Source {
    fn current_span_len(&self) -> Option<usize> {
        if self.exhausted { Some(0) } else { None }
    }

    fn channels(&self) -> rodio::ChannelCount {
        self.channels
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::from_millis(self.duration_ms))
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), rodio::source::SeekError> {
        self.seek_source(position)
    }
}

fn seek_error() -> rodio::source::SeekError {
    rodio::source::SeekError::Other(Arc::new(io::Error::other("media seek failed")))
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../spikes/audio/fixtures")
            .join(name)
    }

    fn test_track(name: &str) -> LocalTrack {
        LocalTrack::new(
            format!("fixture-{name}"),
            name,
            None,
            None,
            None,
            fixture(name),
            1_000,
        )
        .expect("valid fixture track")
    }

    #[test]
    fn local_source_direct_symphonia_streams_six_formats_with_bounded_packet_memory() {
        for name in [
            "tone.mp3",
            "tone.flac",
            "tone.m4a",
            "tone.aac",
            "tone.wav",
            "tone.ogg",
        ] {
            let signals = Arc::new(EngineSignals::default());
            let mut source =
                Symphonia06Source::open(&test_track(name), Uuid::now_v7(), Arc::clone(&signals))
                    .unwrap_or_else(|error| {
                        panic!("direct Symphonia 0.6 fixture {name}: {error:?}")
                    });
            let maximum = source.sample_rate.get() as usize
                * source.channels.get() as usize
                * MAX_PACKET_SECONDS;
            assert!(source.by_ref().take(4_096).count() > 0);
            assert!(source.samples.len() <= maximum);
            source
                .try_seek(Duration::from_millis(50))
                .expect("bounded direct seek");
            assert!(source.take(256).count() > 0);
        }
    }

    #[test]
    fn local_source_direct_symphonia_rejects_corrupt_input_without_opening_output() {
        let result = Symphonia06Source::open(
            &test_track("corrupt.mp3"),
            Uuid::now_v7(),
            Arc::new(EngineSignals::default()),
        );
        assert!(matches!(result, Err(AudioEngineError::MediaInvalid)));
    }
}
