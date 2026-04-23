pub mod theme;
pub mod frame;
pub mod layout;
pub mod snap;
pub mod widgets;
pub mod dock;
pub mod oscillators;
pub mod modulation;
pub mod keyboard;
pub mod sequencer_ui;
pub mod arp_walker;
pub mod fx_chain;
pub mod scope;
pub mod patch_browser;
pub mod midi;

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
