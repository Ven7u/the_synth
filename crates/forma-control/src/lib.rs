pub mod event;
pub mod midi;
pub mod protocol;
pub mod source;

pub use event::{make_control_channel, ControlEvent, ControlReceiver, ControlSender};
pub use protocol::{all_params, Command, ParamDescriptor, ParamId, ParamKind};
pub use source::ControlSource;

/// Convert a note name and octave to a MIDI note number.
///
/// ```
/// use forma_control::midi_note;
/// assert_eq!(midi_note!(A, 4), 69);
/// assert_eq!(midi_note!(C, 4), 60);
/// ```
#[macro_export]
macro_rules! midi_note {
    (C,  $oct:expr) => {
        (($oct as u8 + 1) * 12)
    };
    (Cs, $oct:expr) => {
        (($oct as u8 + 1) * 12 + 1)
    };
    (Db, $oct:expr) => {
        (($oct as u8 + 1) * 12 + 1)
    };
    (D,  $oct:expr) => {
        (($oct as u8 + 1) * 12 + 2)
    };
    (Ds, $oct:expr) => {
        (($oct as u8 + 1) * 12 + 3)
    };
    (Eb, $oct:expr) => {
        (($oct as u8 + 1) * 12 + 3)
    };
    (E,  $oct:expr) => {
        (($oct as u8 + 1) * 12 + 4)
    };
    (F,  $oct:expr) => {
        (($oct as u8 + 1) * 12 + 5)
    };
    (Fs, $oct:expr) => {
        (($oct as u8 + 1) * 12 + 6)
    };
    (Gb, $oct:expr) => {
        (($oct as u8 + 1) * 12 + 6)
    };
    (G,  $oct:expr) => {
        (($oct as u8 + 1) * 12 + 7)
    };
    (Gs, $oct:expr) => {
        (($oct as u8 + 1) * 12 + 8)
    };
    (Ab, $oct:expr) => {
        (($oct as u8 + 1) * 12 + 8)
    };
    (A,  $oct:expr) => {
        (($oct as u8 + 1) * 12 + 9)
    };
    (As, $oct:expr) => {
        (($oct as u8 + 1) * 12 + 10)
    };
    (Bb, $oct:expr) => {
        (($oct as u8 + 1) * 12 + 10)
    };
    (B,  $oct:expr) => {
        (($oct as u8 + 1) * 12 + 11)
    };
}
