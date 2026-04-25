//! Denormal / subnormal float protection for the audio thread.
//!
//! Subnormal floats arise naturally in reverb tails, long decays, and any
//! feedback path where values asymptote toward zero. On Intel/AMD CPUs,
//! operations on subnormals can be **10–100× slower** than normal FP ops —
//! long enough to break realtime. ARM (Apple Silicon, Neoverse) has a
//! similar penalty on at least some cores.
//!
//! The fix is a one-time CPU-mode flip per audio thread: enable flush-to-zero
//! (FTZ — results that would be subnormal are clamped to zero) and
//! denormals-are-zero (DAZ — inputs that are subnormal are treated as zero).
//! Zero audible effect; decisively eliminates the worst-case CPU cliff.
//!
//! Call [`enable_ftz_on_current_thread`] once from every thread that runs
//! DSP code — typically the cpal audio callback and the benchmark harness.
//! Calling it multiple times is fine; it just sets a thread-local CPU flag.

/// Enable flush-to-zero / denormals-are-zero on the calling thread's FPU.
///
/// No-op on targets where the flag doesn't exist (WASM, etc.); safe to call
/// unconditionally.
#[inline]
pub fn enable_ftz_on_current_thread() {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        use core::arch::x86_64::{_mm_getcsr, _mm_setcsr};
        // MXCSR bit 15: FTZ (flush-to-zero), bit 6: DAZ (denormals-are-zero)
        const FTZ: u32 = 0x8000;
        const DAZ: u32 = 0x0040;
        _mm_setcsr((_mm_getcsr() | FTZ) | DAZ);
    }

    #[cfg(target_arch = "x86")]
    unsafe {
        use core::arch::x86::{_mm_getcsr, _mm_setcsr};
        const FTZ: u32 = 0x8000;
        const DAZ: u32 = 0x0040;
        _mm_setcsr((_mm_getcsr() | FTZ) | DAZ);
    }

    #[cfg(target_arch = "aarch64")]
    unsafe {
        // ARMv8-A FPCR bit 24 = FZ (flush-to-zero for single-precision).
        // Bit 19 = FZ16 (half-precision, not relevant). Bit 23 = DN
        // (default-NaN). We only need FZ. There's no separate "denormals-
        // are-zero" on ARM — FZ covers both input and output subnormals.
        let mut fpcr: u64;
        core::arch::asm!("mrs {0}, fpcr", out(reg) fpcr, options(nomem, nostack));
        fpcr |= 1u64 << 24;
        core::arch::asm!("msr fpcr, {0}", in(reg) fpcr, options(nomem, nostack));
    }

    // Other architectures: no-op. The global path in IEEE-754 is still
    // correct; it's just not fast on subnormals. Acceptable.
}
