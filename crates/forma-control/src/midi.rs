//! MIDI input engine.
//!
//! `MidiEngine` opens a midir input port and forwards parsed messages to the
//! UI thread via an `mpsc` channel.  The UI thread calls `drain()` each frame
//! and dispatches events to voice_on / voice_off / parameter updates.
//!
//! Usage:
//!   let engine = MidiEngine::new();          // always succeeds (no port open yet)
//!   engine.list_ports()                      // → Vec<String>
//!   engine.connect(port_index)?;             // open a port; can be called again to switch
//!   engine.disconnect();                     // close current port
//!   for ev in engine.drain() { ... }         // call every UI frame

use midir::{MidiInput, MidiInputConnection};
use std::sync::mpsc::{self, Receiver, Sender};

// ---------------------------------------------------------------------------
// MIDI event type
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum MidiEvent {
    NoteOn { channel: u8, note: u8, velocity: u8 },
    NoteOff { channel: u8, note: u8 },
    CC { channel: u8, cc: u8, value: u8 },
    PitchBend { channel: u8, value: f32 },      // -1.0 … +1.0
    Aftertouch { channel: u8, value: u8 },      // channel pressure 0–127
    ProgramChange { channel: u8, program: u8 }, // 0–127
}

// ---------------------------------------------------------------------------
// CC → parameter mapping (standard GM / conventional assignments)
// ---------------------------------------------------------------------------

/// Human-readable name for a CC number, for UI display.
pub fn cc_name(cc: u8) -> &'static str {
    match cc {
        1 => "Mod Wheel",
        7 => "Volume",
        10 => "Pan",
        11 => "Expression",
        28 => "Data Encoder",
        46 => "Prev",
        47 => "Next",
        64 => "Sustain Pedal",
        71 => "Resonance",
        74 => "Cutoff",
        91 => "Reverb",
        _ => "CC",
    }
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

pub struct MidiEngine {
    tx: Sender<MidiEvent>,
    rx: Receiver<MidiEvent>,
    /// Currently open connection (kept alive by ownership).
    _connection: Option<MidiInputConnection<()>>,
    /// Cached port list from the last `list_ports()` call.
    pub port_names: Vec<String>,
    /// Index of the currently open port (None = disconnected).
    pub connected_port: Option<usize>,
}

impl Default for MidiEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl MidiEngine {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx,
            _connection: None,
            port_names: Vec::new(),
            connected_port: None,
        }
    }

    /// Enumerate available MIDI input ports. Updates `self.port_names`.
    pub fn list_ports(&mut self) -> &[String] {
        let Ok(midi_in) = MidiInput::new("forma_enum") else {
            self.port_names.clear();
            return &self.port_names;
        };
        self.port_names = midi_in
            .ports()
            .iter()
            .map(|p| midi_in.port_name(p).unwrap_or_else(|_| "Unknown".into()))
            .collect();
        &self.port_names
    }

    /// Open the port at `index` (from the last `list_ports()` call).
    /// Closes any previously open port first.
    pub fn connect(&mut self, index: usize) -> anyhow::Result<()> {
        // Drop the existing connection first so the port is free.
        self._connection = None;
        self.connected_port = None;

        let midi_in = MidiInput::new("forma_in")?;
        let ports = midi_in.ports();
        let port = ports
            .get(index)
            .ok_or_else(|| anyhow::anyhow!("MIDI port index {index} out of range"))?;

        let tx = self.tx.clone();

        let conn = midi_in
            .connect(
                port,
                "forma_conn",
                move |_stamp, msg, _| {
                    if let Some(ev) = parse_midi(msg) {
                        let _ = tx.send(ev);
                    }
                },
                (),
            )
            .map_err(|e| anyhow::anyhow!("MIDI connect failed: {}", e.kind()))?;

        self._connection = Some(conn);
        self.connected_port = Some(index);
        Ok(())
    }

    /// Close the current port.
    pub fn disconnect(&mut self) {
        self._connection = None;
        self.connected_port = None;
    }

    /// Drain all pending MIDI events accumulated since the last call.
    /// Call this once per UI frame.
    pub fn drain(&self) -> Vec<MidiEvent> {
        self.rx.try_iter().collect()
    }
}

// ---------------------------------------------------------------------------
// Raw MIDI parser
// ---------------------------------------------------------------------------

fn parse_midi(msg: &[u8]) -> Option<MidiEvent> {
    if msg.is_empty() {
        return None;
    }
    let status = msg[0];
    let channel = status & 0x0F;
    let kind = status >> 4;

    match kind {
        0x9 if msg.len() >= 3 => {
            let note = msg[1];
            let velocity = msg[2];
            if velocity == 0 {
                // Note On with velocity 0 is a Note Off per MIDI spec.
                Some(MidiEvent::NoteOff { channel, note })
            } else {
                Some(MidiEvent::NoteOn {
                    channel,
                    note,
                    velocity,
                })
            }
        }
        0x8 if msg.len() >= 3 => Some(MidiEvent::NoteOff {
            channel,
            note: msg[1],
        }),
        0xB if msg.len() >= 3 => Some(MidiEvent::CC {
            channel,
            cc: msg[1],
            value: msg[2],
        }),
        0xC if msg.len() >= 2 => Some(MidiEvent::ProgramChange {
            channel,
            program: msg[1],
        }),
        0xD if msg.len() >= 2 => Some(MidiEvent::Aftertouch {
            channel,
            value: msg[1],
        }),
        0xE if msg.len() >= 3 => {
            // Pitch bend: 14-bit value, centre = 0x2000
            let lsb = msg[1] as u16;
            let msb = msg[2] as u16;
            let raw = (msb << 7) | lsb;
            let value = (raw as f32 - 8192.0) / 8192.0;
            Some(MidiEvent::PitchBend { channel, value })
        }
        _ => None,
    }
}
