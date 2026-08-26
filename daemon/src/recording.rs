//! Per-App/Per-Group Audio Recording
//! (NewImprovenments/09-per-app-recording.md): captures a group's post-DSP
//! signal to a WAV file without touching its normal playback route.
//!
//! Scope decision, disclosed rather than silent: this codebase's DSP/mixing
//! is per **group**, not per individual app stream (same architectural fact
//! `profiles.rs`'s module doc explains) — so "per-app recording" here means
//! recording the group an app is routed into, which for an app in its own
//! dedicated group is exactly per-app, and for an app sharing a group (e.g.
//! several apps in "Default") records that group's mixed signal, same as
//! everything else in this codebase already treats a group as the unit of
//! processing. Format support is WAV/PCM only (spec's own "for reliability"
//! tier) — FLAC and Opus are explicitly listed by the spec as heavier/later
//! additions and are not implemented (no fake encoder standing in).
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
}

pub struct RecordingSession {
    pub id: u32,
    pub group_id: u32,
    pub path: PathBuf,
    pub format: RecordingFormat,
    started_at: Instant,
    /// Set by the control thread to ask the writer thread to finalize and
    /// exit; the writer thread only ever reads it.
    stop_requested: Arc<AtomicBool>,
    frames_written_bits: Arc<AtomicU64>,
    writer_thread: Option<std::thread::JoinHandle<()>>,
}

impl RecordingSession {
    pub fn elapsed_secs(&self) -> f32 {
        self.started_at.elapsed().as_secs_f32()
    }

    pub fn frames_written(&self) -> u64 {
        self.frames_written_bits.load(Ordering::Relaxed)
    }

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
pub fn start(id: u32, group_id: u32, path: PathBuf, format: RecordingFormat) -> Result<(RecordingSession, rtrb::Producer<f32>), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    warn_if_low_disk_space(&path);

    let (producer, consumer) = rtrb::RingBuffer::<f32>::new(RING_CAPACITY);
    let stop_requested = Arc::new(AtomicBool::new(false));
    let frames_written_bits = Arc::new(AtomicU64::new(0));

    let tmp_path = tmp_path_for(&path);
    let spec = hound::WavSpec {
        channels: CHANNELS,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: match format {
            RecordingFormat::WavPcm16 => 16,
            RecordingFormat::WavFloat32 => 32,
        },
        sample_format: match format {
            RecordingFormat::WavPcm16 => hound::SampleFormat::Int,
            RecordingFormat::WavFloat32 => hound::SampleFormat::Float,
        },
    };
    let writer = hound::WavWriter::create(&tmp_path, spec).map_err(|e| e.to_string())?;

    let thread_stop = stop_requested.clone();
    let thread_frames = frames_written_bits.clone();
    let thread_tmp_path = tmp_path.clone();
    let thread_final_path = path.clone();
    let writer_thread = std::thread::spawn(move || {
        run_writer_thread(consumer, writer, format, thread_stop, thread_frames, thread_tmp_path, thread_final_path);
    });

    Ok((
        RecordingSession { id, group_id, path, format, started_at: Instant::now(), stop_requested, frames_written_bits, writer_thread: Some(writer_thread) },
        producer,
    ))
}

fn tmp_path_for(path: &std::path::Path) -> PathBuf {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    PathBuf::from(tmp)
}

/// Drains the ring buffer and writes samples until told to stop *and* the
/// buffer runs dry, then finalizes the WAV header and atomically renames
/// the temp file into place. Runs entirely off the PipeWire mainloop
/// thread — every operation here (file I/O, sleeping) would be a real-time
/// safety violation if it ran on the audio callback's thread instead.
fn run_writer_thread(
    mut consumer: rtrb::Consumer<f32>,
    mut writer: hound::WavWriter<std::io::BufWriter<std::fs::File>>,
    format: RecordingFormat,
    stop_requested: Arc<AtomicBool>,
    frames_written_bits: Arc<AtomicU64>,
    tmp_path: PathBuf,
    final_path: PathBuf,
) {
    let mut samples_written: u64 = 0;
    loop {
        match consumer.pop() {
            Ok(sample) => {
                let result = match format {
                    RecordingFormat::WavPcm16 => writer.write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16),
                    RecordingFormat::WavFloat32 => writer.write_sample(sample),
                };
                if let Err(err) = result {
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
        tracing::error!(%err, "recording: failed to finalize WAV file");
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
        let (session, mut producer) = start(1, 0, path.clone(), RecordingFormat::WavFloat32).expect("start recording");
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
}
