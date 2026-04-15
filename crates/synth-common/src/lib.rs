pub mod beat_clock;
pub mod clock_division;
pub use beat_clock::{BeatClock, BeatClockShared, BeatEvents, BeatPosition};
pub use clock_division::ClockDivision;

use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub struct RestartBatch<const TRACK_COUNT: usize> {
    pub arp: [bool; TRACK_COUNT],
    pub walker: [bool; TRACK_COUNT],
}

impl<const TRACK_COUNT: usize> RestartBatch<TRACK_COUNT> {
    pub fn empty() -> Self {
        Self {
            arp: [false; TRACK_COUNT],
            walker: [false; TRACK_COUNT],
        }
    }

    pub fn all() -> Self {
        Self {
            arp: [true; TRACK_COUNT],
            walker: [true; TRACK_COUNT],
        }
    }
}

#[derive(Clone, Debug)]
pub struct SyncTransport<const TRACK_COUNT: usize> {
    pub clock_sync_enabled: bool,
    pub bar_quantize_start: bool,
    pub playing: bool,
    pub bpm: u32,
    pub current_step: usize,
    pub last_tick: Instant,
    arp_restart_pending: [bool; TRACK_COUNT],
    walker_restart_pending: [bool; TRACK_COUNT],
}

impl<const TRACK_COUNT: usize> SyncTransport<TRACK_COUNT> {
    pub fn new(default_bpm: u32) -> Self {
        Self {
            clock_sync_enabled: true,
            bar_quantize_start: false,
            playing: false,
            bpm: default_bpm,
            current_step: 0,
            last_tick: Instant::now(),
            arp_restart_pending: [false; TRACK_COUNT],
            walker_restart_pending: [false; TRACK_COUNT],
        }
    }

    pub fn sync_now(&mut self) -> RestartBatch<TRACK_COUNT> {
        self.current_step = 0;
        self.last_tick = Instant::now();
        self.arp_restart_pending.fill(false);
        self.walker_restart_pending.fill(false);
        RestartBatch::all()
    }

    pub fn set_playing(&mut self, playing: bool) -> RestartBatch<TRACK_COUNT> {
        self.playing = playing;
        if self.playing {
            self.last_tick = Instant::now();
            if self.clock_sync_enabled && self.bar_quantize_start && self.current_step == 0 {
                let mut out = RestartBatch::empty();
                for ti in 0..TRACK_COUNT {
                    if self.arp_restart_pending[ti] {
                        out.arp[ti] = true;
                        self.arp_restart_pending[ti] = false;
                    }
                    if self.walker_restart_pending[ti] {
                        out.walker[ti] = true;
                        self.walker_restart_pending[ti] = false;
                    }
                }
                return out;
            }
            RestartBatch::empty()
        } else {
            self.arp_restart_pending.fill(false);
            self.walker_restart_pending.fill(false);
            RestartBatch::empty()
        }
    }

    pub fn set_clock_sync(&mut self, enabled: bool) {
        self.clock_sync_enabled = enabled;
        if !enabled {
            self.arp_restart_pending.fill(false);
            self.walker_restart_pending.fill(false);
        }
    }

    pub fn schedule_or_restart_arp(&mut self, track: usize) -> bool {
        if self.clock_sync_enabled && self.bar_quantize_start && self.playing {
            self.arp_restart_pending[track] = true;
            false
        } else {
            true
        }
    }

    pub fn schedule_or_restart_walker(&mut self, track: usize) -> bool {
        if self.clock_sync_enabled && self.bar_quantize_start && self.playing {
            self.walker_restart_pending[track] = true;
            false
        } else {
            true
        }
    }

    pub fn tick(&mut self) -> RestartBatch<TRACK_COUNT> {
        if !self.playing {
            return RestartBatch::empty();
        }
        let step_dur = self.step_duration();
        if self.last_tick.elapsed() < step_dur {
            return RestartBatch::empty();
        }
        self.last_tick = Instant::now();
        self.current_step = (self.current_step + 1) % 8;
        if self.current_step != 0 {
            return RestartBatch::empty();
        }

        let mut out = RestartBatch::empty();
        for ti in 0..TRACK_COUNT {
            if self.arp_restart_pending[ti] {
                out.arp[ti] = true;
                self.arp_restart_pending[ti] = false;
            }
            if self.walker_restart_pending[ti] {
                out.walker[ti] = true;
                self.walker_restart_pending[ti] = false;
            }
        }
        out
    }

    pub fn bpm_f32(&self) -> f32 {
        self.bpm as f32
    }

    fn step_duration(&self) -> Duration {
        Duration::from_millis(60_000 / self.bpm as u64 / 2)
    }
}
