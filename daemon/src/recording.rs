//! Per-App/Per-Group Audio Recording
//! (NewImprovenments/09-per-app-recording.md): captures a group's post-DSP
//! signal to a WAV or FLAC file without touching its normal playback route.
//!
//! Scope decision, disclosed rather than silent: this codebase's DSP/mixing
//! is per **group**, not per individual app stream (same architectural fact
//! `profiles.rs`'s module doc explains) — so "per-app recording" here means
//! recording the group an app is routed into, which for an app in its own
//! dedicated group (e.g. its "Own DSP" sub-group) is exactly per-app, and for an app sharing a group (e.g.
//! several apps in "Default") records that group's mixed signal, same as
//! everything else in this codebase already treats a group as the unit of
//! processing. Formats: WAV (16-bit PCM or 32-bit float) and FLAC (24-bit,
//! pure-Rust `flacenc`, encoded block by block so memory stays flat on long
//! recordings). Opus isn't implemented.
//!
//! Real-time safety: the audio callback (`pw.rs`) only ever does a
//! lock-free, non-blocking `Producer::push` into a ring buffer — the same
//! pattern already used for the capture->playback bridge and for each
//! output route's fan-out. All file I/O (`hound::WavWriter`, `fsync`,
//! `rename`) happens on a dedicated writer thread spawned when recording
//! starts, never on the PipeWire mainloop thread.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecordingFormat {
    /// 16-bit PCM — smaller files, universally compatible.
    WavPcm16,
    /// 32-bit IEEE float — lossless relative to this daemon's own internal
    /// f32 pipeline, larger files.
    WavFloat32,
    /// 24-bit FLAC — lossless at 24 bits, roughly half the size of WAV.
    Flac24,
}

/// One open output file; the writer thread feeds it interleaved stereo.
enum SampleWriter {
    Wav(hound::WavWriter<std::io::BufWriter<std::fs::File>>, RecordingFormat),
    // Boxed: the FLAC writer is many times the size of the WAV one.
    Flac(Box<FlacWriter>),
}

impl SampleWriter {
    fn create(path: &std::path::Path, format: RecordingFormat) -> Result<Self, String> {
        let wav = |bits, sample_format| {
            let spec = hound::WavSpec { channels: CHANNELS, sample_rate: SAMPLE_RATE, bits_per_sample: bits, sample_format };
            hound::WavWriter::create(path, spec).map(|w| SampleWriter::Wav(w, format)).map_err(|e| e.to_string())
        };
        match format {
            RecordingFormat::WavPcm16 => wav(16, hound::SampleFormat::Int),
            RecordingFormat::WavFloat32 => wav(32, hound::SampleFormat::Float),
            RecordingFormat::Flac24 => FlacWriter::create(path).map(|f| SampleWriter::Flac(Box::new(f))),
        }
    }

    fn write(&mut self, sample: f32) -> Result<(), String> {
        match self {
            SampleWriter::Wav(w, RecordingFormat::WavPcm16) => w.write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16),
            SampleWriter::Wav(w, _) => w.write_sample(sample),
            SampleWriter::Flac(f) => return f.write(sample),
        }
        .map_err(|e| e.to_string())
    }

    fn finalize(self) -> Result<(), String> {
        match self {
            SampleWriter::Wav(w, _) => w.finalize().map_err(|e| e.to_string()),
            SampleWriter::Flac(f) => f.finalize(),
        }
    }
}

const FLAC_BITS: usize = 24;
const FLAC_BLOCK: usize = 4096;

/// Streaming FLAC: a placeholder STREAMINFO header goes out first, each
/// full block is encoded and appended as it fills, and `finalize` seeks
/// back to rewrite the header with the real totals and MD5 (STREAMINFO is
/// a fixed 42 bytes, so the rewrite never shifts the frames).
struct FlacWriter {
    file: std::io::BufWriter<std::fs::File>,
    info: flacenc::component::StreamInfo,
    config: flacenc::error::Verified<flacenc::config::Encoder>,
    framebuf: flacenc::source::FrameBuf,
    /// Running MD5 of the input, for STREAMINFO.
    context: flacenc::source::Context,
    pending: Vec<i32>,
    frame_number: usize,
}

impl FlacWriter {
    fn create(path: &std::path::Path) -> Result<Self, String> {
        use flacenc::error::Verify;
        let mut info = flacenc::component::StreamInfo::new(SAMPLE_RATE as usize, CHANNELS as usize, FLAC_BITS).map_err(|e| format!("{e:?}"))?;
        info.set_block_sizes(FLAC_BLOCK, FLAC_BLOCK).map_err(|e| format!("{e:?}"))?;
        let mut writer = Self {
            file: std::io::BufWriter::new(std::fs::File::create(path).map_err(|e| e.to_string())?),
            info,
            config: flacenc::config::Encoder::default().into_verified().map_err(|(_, e)| format!("{e:?}"))?,
            framebuf: flacenc::source::FrameBuf::with_size(CHANNELS as usize, FLAC_BLOCK).map_err(|e| format!("{e:?}"))?,
            context: flacenc::source::Context::new(FLAC_BITS, CHANNELS as usize),
            pending: Vec::with_capacity(FLAC_BLOCK * CHANNELS as usize),
            frame_number: 0,
        };
        writer.write_header()?;
        Ok(writer)
    }

    fn write_header(&mut self) -> Result<(), String> {
        use flacenc::component::BitRepr;
        use std::io::Write;
        let header = flacenc::component::Stream::with_stream_info(self.info.clone());
        let mut sink = flacenc::bitsink::ByteSink::new();
        header.write(&mut sink).map_err(|e| format!("{e:?}"))?;
        self.file.write_all(sink.as_slice()).map_err(|e| e.to_string())
    }

    fn write(&mut self, sample: f32) -> Result<(), String> {
        const MAX: f32 = ((1 << (FLAC_BITS - 1)) - 1) as f32;
        self.pending.push((sample.clamp(-1.0, 1.0) * MAX) as i32);
        if self.pending.len() == FLAC_BLOCK * CHANNELS as usize {
            self.encode_pending()?;
        }
        Ok(())
    }

    fn encode_pending(&mut self) -> Result<(), String> {
        use flacenc::component::BitRepr;
        use flacenc::source::Fill;
        use std::io::Write;
        if self.pending.is_empty() {
            return Ok(());
        }
        (&mut self.framebuf, &mut self.context).fill_interleaved(&self.pending).map_err(|e| format!("{e:?}"))?;
        let frame = flacenc::encode_fixed_size_frame(&self.config, &self.framebuf, self.frame_number, &self.info).map_err(|e| format!("{e:?}"))?;
        self.info.update_frame_info(&frame);
        let mut sink = flacenc::bitsink::ByteSink::new();
        frame.write(&mut sink).map_err(|e| format!("{e:?}"))?;
        self.file.write_all(sink.as_slice()).map_err(|e| e.to_string())?;
        self.pending.clear();
        self.frame_number += 1;
        Ok(())
    }

    fn finalize(mut self) -> Result<(), String> {
        use std::io::{Seek, Write};
        // A trailing odd sample can't form a frame; drop it rather than
        // corrupting channel order.
        self.pending.truncate(self.pending.len() / CHANNELS as usize * CHANNELS as usize);
        self.encode_pending()?;
        // Block sizes as a fixed-block stream declares them (update_frame_info
        // lowered min to the short last block).
        self.info.set_block_sizes(FLAC_BLOCK, FLAC_BLOCK).map_err(|e| format!("{e:?}"))?;
        self.info.set_md5_digest(&self.context.md5_digest());
        self.file.seek(std::io::SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        self.write_header()?;
        self.file.flush().map_err(|e| e.to_string())?;
        self.file.get_ref().sync_all().map_err(|e| e.to_string())
    }
}

pub struct RecordingSession {
    pub group_id: u32,
    started_at: Instant,
    /// Set by the control thread to ask the writer thread to finalize and
    /// exit; the writer thread only ever reads it.
    stop_requested: Arc<AtomicBool>,
    frames_written_bits: Arc<AtomicU64>,
    writer_thread: Option<std::thread::JoinHandle<()>>,
}

impl RecordingSession {
    /// The two live-readable pieces (`pw.rs` mirrors these into its own
    /// `RecordingStatus` so the D-Bus thread can read status without
    /// touching `Engine`, which only the PipeWire mainloop thread owns).
    pub fn started_at(&self) -> Instant {
        self.started_at
    }
    pub fn frames_written_bits(&self) -> Arc<AtomicU64> {
        self.frames_written_bits.clone()
    }

    /// Signals the writer thread to finalize the WAV file (proper header,
    /// atomic rename into place) and waits for it — called for an explicit
    /// D-Bus stop, and from `pw.rs`'s graceful-shutdown path so a SIGTERM
    /// doesn't leave a headerless/truncated file (spec's own "finalize the
    /// file rather than leaving corrupt output"). An unclean kill (SIGKILL,
    /// power loss) can still lose the last unfluhsed samples — documented
    /// limitation, same honesty standard the rest of this daemon holds
    /// persistence claims to.
    pub fn stop_and_finalize(mut self) {
        self.stop_requested.store(true, Ordering::Relaxed);
        if let Some(handle) = self.writer_thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for RecordingSession {
    fn drop(&mut self) {
        self.stop_requested.store(true, Ordering::Relaxed);
        if let Some(handle) = self.writer_thread.take() {
            let _ = handle.join();
        }
    }
}

const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
/// Bounded so a stalled/slow disk applies backpressure by dropping samples
/// (never blocks the real-time producer side) rather than growing without
/// bound — about 2 seconds of stereo f32 headroom.
const RING_CAPACITY: usize = SAMPLE_RATE as usize * CHANNELS as usize * 2;

/// Starts a new recording: creates the ring buffer, spawns the writer
/// thread, and returns both the session handle (for D-Bus status/stop) and
/// the ring buffer's producer half (for `pw.rs` to register into the
/// group's capture-callback fan-out). `path` is the final destination;
/// the writer thread writes to `path` with a `.tmp` suffix and renames on
/// finalize, so a reader never sees a partial file at the real path.
pub fn start(group_id: u32, path: PathBuf, format: RecordingFormat) -> Result<(RecordingSession, rtrb::Producer<f32>), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    warn_if_low_disk_space(&path);

    let (producer, consumer) = rtrb::RingBuffer::<f32>::new(RING_CAPACITY);
    let stop_requested = Arc::new(AtomicBool::new(false));
    let frames_written_bits = Arc::new(AtomicU64::new(0));

    let tmp_path = tmp_path_for(&path);
    let writer = SampleWriter::create(&tmp_path, format)?;

    let thread_stop = stop_requested.clone();
    let thread_frames = frames_written_bits.clone();
    let thread_tmp_path = tmp_path.clone();
    let thread_final_path = path.clone();
    let writer_thread = std::thread::spawn(move || {
        run_writer_thread(consumer, writer, thread_stop, thread_frames, thread_tmp_path, thread_final_path);
    });

    Ok((
        RecordingSession { group_id, started_at: Instant::now(), stop_requested, frames_written_bits, writer_thread: Some(writer_thread) },
        producer,
    ))
}

fn tmp_path_for(path: &std::path::Path) -> PathBuf {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    PathBuf::from(tmp)
}

/// Drains the ring buffer and writes samples until told to stop *and* the
/// buffer runs dry, then finalizes the file header and atomically renames
/// the temp file into place. Runs entirely off the PipeWire mainloop
/// thread — every operation here (file I/O, sleeping) would be a real-time
/// safety violation if it ran on the audio callback's thread instead.
fn run_writer_thread(
    mut consumer: rtrb::Consumer<f32>,
    mut writer: SampleWriter,
    stop_requested: Arc<AtomicBool>,
    frames_written_bits: Arc<AtomicU64>,
    tmp_path: PathBuf,
    final_path: PathBuf,
) {
    let mut samples_written: u64 = 0;
    loop {
        match consumer.pop() {
            Ok(sample) => {
                if let Err(err) = writer.write(sample) {
                    tracing::error!(%err, "recording: failed to write sample, stopping");
                    break;
                }
                samples_written += 1;
                // Frames, not raw samples (a stereo frame = 2 samples) — what
                // the elapsed-time/status readout actually wants.
                frames_written_bits.store(samples_written / CHANNELS as u64, Ordering::Relaxed);
            }
            Err(rtrb::PopError::Empty) => {
                if stop_requested.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }
    if let Err(err) = writer.finalize() {
        tracing::error!(%err, "recording: failed to finalize file");
        return;
    }
    if let Err(err) = std::fs::rename(&tmp_path, &final_path) {
        tracing::error!(%err, path = %final_path.display(), "recording: failed to move finished file into place");
    } else {
        tracing::info!(path = %final_path.display(), "recording finalized");
    }
}

/// Best-effort free-space check (spec's "disk-space warnings") — logs a
/// warning rather than blocking the recording from starting; MTX should
/// not refuse to record just because its own estimate might be wrong.
fn warn_if_low_disk_space(path: &std::path::Path) {
    let dir = path.parent().unwrap_or(std::path::Path::new("."));
    let Some(free_bytes) = free_bytes(dir) else { return };
    const LOW_SPACE_THRESHOLD_BYTES: u64 = 200 * 1024 * 1024;
    if free_bytes < LOW_SPACE_THRESHOLD_BYTES {
        tracing::warn!(free_mb = free_bytes / 1024 / 1024, "recording: low disk space at destination");
    }
}

fn free_bytes(dir: &std::path::Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut stat) != 0 {
            return None;
        }
        Some(stat.f_bavail as u64 * stat.f_frsize as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tmp_path_appends_suffix() {
        assert_eq!(tmp_path_for(std::path::Path::new("/x/rec.wav")), PathBuf::from("/x/rec.wav.tmp"));
    }

    #[test]
    fn record_and_finalize_produces_a_valid_wav_file() {
        let dir = std::env::temp_dir().join(format!("mtx_rec_test_{}", std::process::id()));
        let path = dir.join("out.wav");
        let (session, mut producer) = start(0, path.clone(), RecordingFormat::WavFloat32).expect("start recording");
        for i in 0..2000 {
            let _ = producer.push((i as f32 * 0.001).sin());
        }
        drop(producer); // no more samples coming
        std::thread::sleep(std::time::Duration::from_millis(50));
        session.stop_and_finalize();

        assert!(path.exists(), "finalized file should exist at the real path, not just .tmp");
        let reader = hound::WavReader::open(&path).expect("finalized file should be a valid WAV");
        assert!(reader.duration() > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn flac_round_trips_through_an_independent_decoder() {
        let dir = std::env::temp_dir().join(format!("mtx_flac_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.flac");
        // Two full blocks plus a short tail, to exercise the partial last frame.
        let frames = FLAC_BLOCK * 2 + 1000;
        let input: Vec<f32> = (0..frames * 2).map(|i| (i as f32 * 0.01).sin() * 0.5).collect();
        let mut w = SampleWriter::create(&path, RecordingFormat::Flac24).unwrap();
        for &x in &input {
            w.write(x).unwrap();
        }
        w.finalize().unwrap();

        let mut reader = claxon::FlacReader::open(&path).expect("valid FLAC");
        let info = reader.streaminfo();
        assert_eq!((info.sample_rate, info.channels, info.bits_per_sample), (SAMPLE_RATE, 2, 24));
        assert_eq!(info.samples, Some(frames as u64));
        assert_ne!(info.md5sum, [0; 16], "MD5 should be set");
        let decoded: Vec<i32> = reader.samples().map(|s| s.unwrap()).collect();
        let max = ((1 << 23) - 1) as f32;
        assert_eq!(decoded.len(), input.len());
        assert!(decoded.iter().zip(&input).all(|(&d, &x)| d == (x * max) as i32), "lossless at 24 bits");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
