pub mod biquad;
pub mod delay;
pub mod dynamics;
pub mod smoothed;
pub mod spectrum;

/// Clamp a number from a D-Bus caller into `lo..=hi`. NaN becomes `lo`:
/// `f32::clamp` passes NaN straight through, and one NaN gain or a `q` of 0
/// turns a group's filter state into NaN for good, which the final
/// `clamp(-1, 1)` also lets through to the speakers.
pub fn clamp_input(v: f32, lo: f32, hi: f32) -> f32 {
    if v.is_nan() { lo } else { v.clamp(lo, hi) }
}

/// Make float maths on this thread flush denormals to zero (x86 FTZ + DAZ,
/// Arm FZ). Called at the top of every realtime `process` callback.
///
/// When an app goes silent, IIR filter state decays toward zero and passes
/// through the denormal range, where x86 floating point is many times
/// slower (14x measured for the EQ biquads). A group's callback then
/// outlasted the audio period, the data-loop thread stopped sleeping, and
/// after 200 ms of realtime CPU the kernel SIGKILLed the daemon
/// (RLIMIT_RTTIME): no log line, just `status=9/KILL`, dozens of times a day.
/// The bits only touch the calling thread and are cheap to set every cycle.
#[inline]
pub fn no_denormals() {
    #[cfg(target_arch = "x86_64")]
    #[allow(deprecated)] // the intrinsics are deprecated in favour of inline asm; they still do exactly this
    unsafe {
        use std::arch::x86_64::{_mm_getcsr, _mm_setcsr};
        _mm_setcsr(_mm_getcsr() | 0x8040); // FTZ (bit 15) | DAZ (bit 6)
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let fpcr: u64;
        std::arch::asm!("mrs {}, fpcr", out(reg) fpcr);
        std::arch::asm!("msr fpcr, {}", in(reg) fpcr | 1 << 24); // FZ
    }
}

#[cfg(test)]
mod tests {
    use super::biquad::{Biquad, EqBand};

    #[test]
    fn clamp_input_turns_nan_into_the_floor() {
        assert_eq!(super::clamp_input(f32::NAN, 0.1, 10.0), 0.1);
        assert_eq!(super::clamp_input(f32::INFINITY, 0.0, 4.0), 4.0);
        assert_eq!(super::clamp_input(0.0, 0.1, 10.0), 0.1);
        assert_eq!(super::clamp_input(2.0, 0.0, 4.0), 2.0);
    }

    #[test]
    fn filters_never_output_denormals_after_no_denormals() {
        super::no_denormals();
        let mut f = Biquad::default();
        f.set_peaking(&EqBand { freq_hz: 100.0, gain_db: 9.0, q: 1.0, enabled: true }, 48_000.0);
        for i in 0..4_800 { f.process((i as f32 * 0.05).sin() * 0.5); }
        // seconds of silence: without FTZ the state decays through the denormal range
        for _ in 0..48_000 * 5 {
            let y = f.process(0.0);
            assert!(!y.is_subnormal(), "denormal output {y:e}");
        }
    }
}
