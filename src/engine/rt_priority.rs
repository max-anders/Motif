//! Realtime scheduling for the cpal audio callback thread.
//!
//! Two things have to happen together, and shipping only the first one turns
//! every xrun into a crash:
//!
//! 1. Promote the thread out of SCHED_OTHER, so the callback stops losing the
//!    CPU to the compositor, a browser, our own UI thread, or another DAW whose
//!    audio threads are realtime.
//! 2. Survive `RLIMIT_RTTIME`. Promotion arms that limit, and the kernel raises
//!    SIGXCPU when a realtime thread burns more CPU than the limit allows
//!    without blocking. SIGXCPU's default action is terminate-with-core, so an
//!    unhandled overrun kills the process outright.
//!
//! `audio_thread_priority` sets the soft limit to a single buffer period
//! (10.7 ms at 512 frames / 48 kHz) and documents that the host is expected to
//! catch SIGXCPU. One period is far too tight to treat as fatal: a debug build
//! with a heavy plugin overruns a period routinely, and that is an xrun - a
//! click - not a runaway thread. So this module raises the soft limit to
//! something that only a genuine spin can reach, installs a handler so the
//! signal is survivable, and steps the thread back down to normal priority if it
//! ever fires. Glitchy audio beats a lost session.

use audio_thread_priority::{promote_current_thread_to_real_time, RtPriorityHandle};

/// Continuous realtime CPU one callback may burn before the kernel flags it.
///
/// Deliberately far above a buffer period (see the module note) and kept under
/// the hard limit so an overrun yields SIGXCPU, which we can act on, rather than
/// SIGKILL, which we cannot.
#[cfg(target_os = "linux")]
const RTTIME_SOFT_LIMIT_US: u64 = 100_000;

/// Scheduling state of the audio thread. Owned by the callback state, so every
/// transition below already runs on the thread being reprioritised - which
/// `demote_current_thread_from_real_time` asserts.
pub(super) struct RtPriorityState {
    status: Status,
}

enum Status {
    /// Nothing attempted yet; the first callback claims priority.
    Unclaimed,
    /// Realtime. The handle is held for exactly as long as the promotion is in
    /// force and dropped when we step back down, so that a future crate version
    /// which reverts on `Drop` would do so at the right moment. Nothing reads it
    /// today - the demote syscall is ours.
    Realtime(#[allow(dead_code)] RtPriorityHandle),
    /// Normal priority: the promotion failed, or an overrun forced a demote.
    /// Terminal - we do not retry, because whatever made the callback overrun
    /// once will still be there on the next block.
    Normal,
}

impl RtPriorityState {
    pub(super) fn new() -> Self {
        Self {
            status: Status::Unclaimed,
        }
    }

    /// Call once per block from the audio callback.
    ///
    /// Claims realtime scheduling on the first block, then does nothing but read
    /// an atomic flag - cheap enough for the hot path. The claim itself performs
    /// blocking D-Bus I/O to rtkit, which is exactly what a callback must never
    /// do, but it happens once, on the first block, before there is any audio
    /// worth protecting.
    pub(super) fn update(&mut self, frames: usize, sample_rate: f32) {
        if matches!(self.status, Status::Unclaimed) {
            self.claim(frames, sample_rate);
        } else if matches!(self.status, Status::Realtime(_)) && overrun::take() {
            self.demote();
        }
    }

    fn claim(&mut self, frames: usize, sample_rate: f32) {
        let sample_rate_hz = sample_rate.round().max(1.0) as u32;

        // Before promoting, not after: the promotion arms RLIMIT_RTTIME, so an
        // overrun is possible from the very next instruction.
        overrun::install_handler();

        self.status = match promote_current_thread_to_real_time(frames as u32, sample_rate_hz) {
            Ok(handle) => {
                overrun::relax_soft_limit();
                // Discard anything raised in the brief window where the crate's
                // one-period limit was still in force.
                overrun::take();
                eprintln!(
                    "Motif audio thread promoted to realtime \
                     ({frames} frames @ {sample_rate_hz} Hz)"
                );
                Status::Realtime(handle)
            }
            Err(error) => {
                eprintln!(
                    "Motif audio thread stayed at normal priority ({error}); \
                     expect xruns under load"
                );
                Status::Normal
            }
        };
    }

    fn demote(&mut self) {
        // Drops the retained handle, which is deliberate: the syscall is ours
        // rather than the crate's (see `overrun::demote_to_normal`).
        self.status = Status::Normal;
        match overrun::demote_to_normal() {
            Ok(()) => eprintln!(
                "Motif audio thread overran its realtime budget; dropped to normal \
                 priority for the rest of the session (expect glitches, not a crash)"
            ),
            Err(error) => eprintln!(
                "Motif audio thread overran its realtime budget and could not be \
                 demoted ({error})"
            ),
        }
    }
}

#[cfg(target_os = "linux")]
mod overrun {
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Raised by the kernel on the offending thread, so the handler runs on the
    /// audio thread and the flag is read by that same thread on its next block.
    static OVERRAN: AtomicBool = AtomicBool::new(false);
    static HANDLER_INSTALLED: AtomicBool = AtomicBool::new(false);

    extern "C" fn on_sigxcpu(_signum: libc::c_int) {
        // A relaxed atomic store is one of the few things that is actually legal
        // in a signal handler. No allocation, no locks, no logging.
        OVERRAN.store(true, Ordering::Relaxed);
    }

    pub(super) fn install_handler() {
        if HANDLER_INSTALLED.swap(true, Ordering::Relaxed) {
            return;
        }
        // SAFETY: `on_sigxcpu` only performs a relaxed atomic store, and the
        // action is zeroed before use so no field is left uninitialised.
        let installed = unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = on_sigxcpu as extern "C" fn(libc::c_int) as libc::sighandler_t;
            // SA_RESTART so the signal cannot make ALSA's write/poll fail with
            // EINTR and take the stream down instead of the process.
            action.sa_flags = libc::SA_RESTART;
            libc::sigemptyset(&mut action.sa_mask);
            libc::sigaction(libc::SIGXCPU, &action, std::ptr::null_mut()) == 0
        };
        if !installed {
            eprintln!(
                "Motif could not install a SIGXCPU handler; a callback overrun \
                 will terminate the process"
            );
        }
    }

    /// Widen the window an overrunning callback gets before the kernel complains.
    /// Only the soft limit moves; the hard limit stays where rtkit put it, and
    /// raising that needs CAP_SYS_RESOURCE anyway.
    pub(super) fn relax_soft_limit() {
        // SAFETY: both calls take a pointer to a local `rlimit` that is fully
        // initialised by `getrlimit` before being read.
        unsafe {
            let mut limits: libc::rlimit = std::mem::zeroed();
            if libc::getrlimit(libc::RLIMIT_RTTIME, &mut limits) != 0 {
                return;
            }
            // Stay strictly under the hard limit: reaching that is a SIGKILL we
            // get no say in.
            let ceiling = (limits.rlim_max as u64) / 2;
            let soft = super::RTTIME_SOFT_LIMIT_US.min(ceiling.max(1));
            if soft <= limits.rlim_cur as u64 {
                return;
            }
            limits.rlim_cur = soft as libc::rlim_t;
            libc::setrlimit(libc::RLIMIT_RTTIME, &limits);
        }
    }

    pub(super) fn take() -> bool {
        OVERRAN.swap(false, Ordering::Relaxed)
    }

    /// https://github.com/rust-lang/libc/issues/1511 - libc still does not export it.
    const SCHED_RESET_ON_FORK: libc::c_int = 0x4000_0000;

    /// Step the calling thread back down to normal scheduling.
    ///
    /// Deliberately not `demote_current_thread_from_real_time`: that restores the
    /// policy captured before promotion, which does not carry
    /// SCHED_RESET_ON_FORK. rtkit sets that flag, and an unprivileged thread is
    /// not permitted to clear it, so the call fails with EPERM - silently,
    /// because the crate tests the result for `< 0` while
    /// `pthread_setschedparam` reports failures as a positive errno. The
    /// observable symptom was a demote that logged success while `chrt` still
    /// reported SCHED_RR. Preserving the flag is what makes it actually take.
    pub(super) fn demote_to_normal() -> Result<(), std::io::Error> {
        // SAFETY: an all-zero `sched_param` is valid for SCHED_OTHER, which
        // ignores the priority field entirely.
        let errno = unsafe {
            let param: libc::sched_param = std::mem::zeroed();
            libc::pthread_setschedparam(
                libc::pthread_self(),
                libc::SCHED_OTHER | SCHED_RESET_ON_FORK,
                &param,
            )
        };
        if errno == 0 {
            Ok(())
        } else {
            Err(std::io::Error::from_raw_os_error(errno))
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod overrun {
    /// RLIMIT_RTTIME is Linux-only; nothing to arm or catch elsewhere.
    pub(super) fn install_handler() {}
    pub(super) fn relax_soft_limit() {}
    pub(super) fn take() -> bool {
        false
    }
    pub(super) fn demote_to_normal() -> Result<(), std::io::Error> {
        Ok(())
    }
}
