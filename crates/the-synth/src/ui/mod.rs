pub mod arp_walker;
pub mod dock;
pub mod frame;
pub mod fx_chain;
pub mod keyboard;
pub mod layout;
pub mod midi;
pub mod modulation;
pub mod oscillators;
pub mod patch_browser;
pub mod scope;
pub mod sequencer_ui;
pub mod snap;
pub mod theme;
pub mod widgets;

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
