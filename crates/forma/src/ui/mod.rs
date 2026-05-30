pub mod arp_walker;
pub mod dock;
pub mod drum_machine_ui;
pub mod eq_ui;
pub mod frame;
pub mod fx_chain;
pub mod history_ui;
pub mod keyboard;
pub mod layout;
#[cfg(feature = "live_rig")]
pub mod live_view;
pub mod metronome;
pub mod midi;
#[cfg(feature = "live_rig")]
pub mod mixer;
pub mod modulation;
pub mod oscillators;
pub mod patch_browser;
pub mod pattern_library;
#[cfg(feature = "live_rig")]
pub mod rig_strip;
pub mod scene_browser;
pub mod scope;
pub mod scope_wgpu;
pub mod sequencer_ui;
pub mod snap;
pub mod theme;
pub mod widgets;

/// MIDI note number → full name with octave, e.g. 60 → "C4".
pub fn midi_note_full(midi: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = (midi as i32 / 12) - 1;
    format!("{}{}", NAMES[(midi % 12) as usize], octave)
}

pub fn midi_note_name(midi: u8) -> &'static str {
    match midi % 12 {
        0 => "C",
        1 => "C#",
        2 => "D",
        3 => "D#",
        4 => "E",
        5 => "F",
        6 => "F#",
        7 => "G",
        8 => "G#",
        9 => "A",
        10 => "A#",
        11 => "B",
        _ => "?",
    }
}
