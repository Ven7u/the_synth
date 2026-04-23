//! Musical clock divisions — shared by BPM-synced delay and BPM-synced LFO.
//!
//! A `ClockDivision` converts to a duration in **beats** (quarter notes).
//! Multiply by `(60.0 / bpm)` to get seconds.
//!
//! Dotted values are 1.5× their base. Triplets are ⅔ of their base.

/// A musical note division relative to a quarter-note beat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ClockDivision {
    // Straight
    Whole = 0,        // 4 beats
    Half = 1,         // 2 beats
    Quarter = 2,      // 1 beat
    Eighth = 3,       // 0.5 beats
    Sixteenth = 4,    // 0.25 beats
    ThirtySecond = 5, // 0.125 beats

    // Dotted (×1.5)
    DottedHalf = 6,      // 3 beats
    DottedQuarter = 7,   // 1.5 beats
    DottedEighth = 8,    // 0.75 beats
    DottedSixteenth = 9, // 0.375 beats

    // Triplets (×2/3)
    HalfTriplet = 10,      // 4/3 beats
    QuarterTriplet = 11,   // 2/3 beats
    EighthTriplet = 12,    // 1/3 beats
    SixteenthTriplet = 13, // 1/6 beats
}

impl ClockDivision {
    /// Duration in **beats** (quarter notes).
    pub fn beats(self) -> f32 {
        match self {
            Self::Whole => 4.0,
            Self::Half => 2.0,
            Self::Quarter => 1.0,
            Self::Eighth => 0.5,
            Self::Sixteenth => 0.25,
            Self::ThirtySecond => 0.125,
            Self::DottedHalf => 3.0,
            Self::DottedQuarter => 1.5,
            Self::DottedEighth => 0.75,
            Self::DottedSixteenth => 0.375,
            Self::HalfTriplet => 4.0 / 3.0,
            Self::QuarterTriplet => 2.0 / 3.0,
            Self::EighthTriplet => 1.0 / 3.0,
            Self::SixteenthTriplet => 1.0 / 6.0,
        }
    }

    /// Duration in **seconds** at `bpm` beats per minute.
    pub fn seconds(self, bpm: f32) -> f32 {
        self.beats() * 60.0 / bpm.max(1.0)
    }

    /// Frequency in **Hz** at `bpm` — useful for LFO rate.
    pub fn hz(self, bpm: f32) -> f32 {
        1.0 / self.seconds(bpm)
    }

    /// Serialize to u8 (matches `#[repr(u8)]`).
    pub fn to_u8(self) -> u8 {
        self as u8
    }

    /// Deserialize from u8. Unknown values fall back to `Quarter`.
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Whole,
            1 => Self::Half,
            2 => Self::Quarter,
            3 => Self::Eighth,
            4 => Self::Sixteenth,
            5 => Self::ThirtySecond,
            6 => Self::DottedHalf,
            7 => Self::DottedQuarter,
            8 => Self::DottedEighth,
            9 => Self::DottedSixteenth,
            10 => Self::HalfTriplet,
            11 => Self::QuarterTriplet,
            12 => Self::EighthTriplet,
            13 => Self::SixteenthTriplet,
            _ => Self::Quarter,
        }
    }

    /// Human-readable label (for UI).
    pub fn label(self) -> &'static str {
        match self {
            Self::Whole => "1/1",
            Self::Half => "1/2",
            Self::Quarter => "1/4",
            Self::Eighth => "1/8",
            Self::Sixteenth => "1/16",
            Self::ThirtySecond => "1/32",
            Self::DottedHalf => "1/2.",
            Self::DottedQuarter => "1/4.",
            Self::DottedEighth => "1/8.",
            Self::DottedSixteenth => "1/16.",
            Self::HalfTriplet => "1/2T",
            Self::QuarterTriplet => "1/4T",
            Self::EighthTriplet => "1/8T",
            Self::SixteenthTriplet => "1/16T",
        }
    }

    /// All variants in display order (for combo boxes).
    pub const ALL: &'static [Self] = &[
        Self::Whole,
        Self::Half,
        Self::Quarter,
        Self::Eighth,
        Self::Sixteenth,
        Self::ThirtySecond,
        Self::DottedHalf,
        Self::DottedQuarter,
        Self::DottedEighth,
        Self::DottedSixteenth,
        Self::HalfTriplet,
        Self::QuarterTriplet,
        Self::EighthTriplet,
        Self::SixteenthTriplet,
    ];
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarter_at_120bpm_is_half_second() {
        let s = ClockDivision::Quarter.seconds(120.0);
        assert!((s - 0.5).abs() < 1e-6, "got {s}");
    }

    #[test]
    fn dotted_eighth_at_120bpm() {
        // 0.75 beats × (60/120) = 0.375 s
        let s = ClockDivision::DottedEighth.seconds(120.0);
        assert!((s - 0.375).abs() < 1e-6, "got {s}");
    }

    #[test]
    fn eighth_triplet_at_120bpm() {
        // 1/3 beat × 0.5 s/beat = 0.1667 s
        let s = ClockDivision::EighthTriplet.seconds(120.0);
        assert!((s - 1.0 / 6.0).abs() < 1e-5, "got {s}");
    }

    #[test]
    fn roundtrip_u8() {
        for &div in ClockDivision::ALL {
            assert_eq!(ClockDivision::from_u8(div.to_u8()), div);
        }
    }
}
