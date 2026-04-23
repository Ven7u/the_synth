//! Headless smoke test for `SynthEngineHandle` + protocol.
//!
//! No cpal, no eframe. Builds the handle directly from an `AudioState` and
//! a control channel, exercises every equivalence path we care about, and
//! asserts descriptor-table invariants.

use std::collections::HashSet;
use std::sync::Arc;

use synth_control::{
    all_params, make_control_channel, Command, ControlEvent, ParamId, ParamKind,
};
use synth_engine::{AudioState, SynthEngineHandle};

fn make_handle() -> (SynthEngineHandle, synth_control::ControlReceiver) {
    let state = Arc::new(AudioState::new());
    let (tx, rx) = make_control_channel(1024);
    (SynthEngineHandle::new(state, tx), rx)
}

#[test]
fn typed_roundtrip_f32() {
    let (h, _rx) = make_handle();
    h.set_filter_cutoff(1234.5);
    assert!((h.filter_cutoff() - 1234.5).abs() < 1e-3);
}

#[test]
fn typed_roundtrip_u8() {
    let (h, _rx) = make_handle();
    h.set_lfo_shape(2);
    assert_eq!(h.lfo_shape(), 2);
    // Clamp beyond legal range.
    h.set_lfo_shape(9);
    assert_eq!(h.lfo_shape(), 2);
}

#[test]
fn typed_roundtrip_bool() {
    let (h, _rx) = make_handle();
    // Default is true.
    assert!(h.limiter_enabled());
    h.set_limiter_enabled(false);
    assert!(!h.limiter_enabled());
}

#[test]
fn apply_equivalence_with_typed_setter() {
    let (h, _rx) = make_handle();
    h.apply(Command::SetParam { id: ParamId::FilterCutoff, value: 3000.0 });
    assert!((h.filter_cutoff() - 3000.0).abs() < 1e-3);

    h.apply(Command::SetParam { id: ParamId::LfoShape, value: 1.0 });
    assert_eq!(h.lfo_shape(), 1);

    h.apply(Command::SetParam { id: ParamId::LimiterEnabled, value: 0.0 });
    assert!(!h.limiter_enabled());
}

#[test]
fn events_land_on_the_channel() {
    let (h, rx) = make_handle();

    h.note_on(60, 100);
    let ev = rx.try_recv().expect("NoteOn should be on the channel");
    match ev {
        ControlEvent::NoteOn { pitch, velocity, track } => {
            assert_eq!(pitch, 60);
            assert_eq!(velocity, 100);
            assert_eq!(track, 0);
        }
        other => panic!("unexpected event: {other:?}"),
    }

    h.note_off(60);
    let ev = rx.try_recv().unwrap();
    assert!(matches!(ev, ControlEvent::NoteOff { pitch: 60, track: 0 }));

    h.arp_restart();
    let ev = rx.try_recv().unwrap();
    assert!(matches!(ev, ControlEvent::ArpRestart { track: 0 }));

    h.walker_restart();
    let ev = rx.try_recv().unwrap();
    assert!(matches!(ev, ControlEvent::WalkerRestart { track: 0 }));

    h.chord_hold(&[60, 64, 67]);
    let ev = rx.try_recv().unwrap();
    match ev {
        ControlEvent::ChordHold { track, notes } => {
            assert_eq!(track, 0);
            assert_eq!(notes, vec![60, 64, 67]);
        }
        other => panic!("expected ChordHold, got {other:?}"),
    }
}

#[test]
fn descriptor_table_invariants() {
    let params = all_params();
    assert!(!params.is_empty(), "descriptor table is empty");

    let mut ids: HashSet<ParamId> = HashSet::new();
    for desc in params {
        assert!(
            ids.insert(desc.id),
            "duplicate ParamId in descriptor table: {:?}",
            desc.id
        );
        assert!(
            desc.min <= desc.default,
            "{:?}: min {} > default {}",
            desc.id, desc.min, desc.default
        );
        assert!(
            desc.default <= desc.max,
            "{:?}: default {} > max {}",
            desc.id, desc.default, desc.max
        );
    }
    assert!(
        params.len() >= 110,
        "expected ≥110 descriptors, got {}",
        params.len()
    );
}

#[test]
fn apply_default_for_every_descriptor() {
    let (h, _rx) = make_handle();

    for desc in all_params() {
        // Dispatching the default must never panic.
        h.apply(Command::SetParam { id: desc.id, value: desc.default });

        // For params we can also read back, confirm semantics.
        if let Some(got) = h.get_by_id(desc.id) {
            match desc.kind {
                ParamKind::Bool => {
                    let want = if desc.default != 0.0 { 1.0 } else { 0.0 };
                    assert!(
                        (got - want).abs() < 1e-3,
                        "{:?}: bool round-trip failed — want {}, got {}",
                        desc.id, want, got
                    );
                }
                ParamKind::Discrete(_) => {
                    let want = desc.default.round();
                    assert!(
                        (got - want).abs() < 1e-3,
                        "{:?}: discrete round-trip failed — want {}, got {}",
                        desc.id, want, got
                    );
                }
                ParamKind::Linear | ParamKind::Log => {
                    assert!(
                        (got - desc.default).abs() < 1e-3 || (got - desc.default).abs() / desc.default.abs().max(1e-6) < 1e-3,
                        "{:?}: numeric round-trip failed — want {}, got {}",
                        desc.id, desc.default, got
                    );
                }
            }
        }
    }
}

#[test]
fn readback_surfaces_return_initial_values() {
    let (h, _rx) = make_handle();
    // Cursors and meters are initialised to 0.0 on a fresh state.
    assert_eq!(h.amp_cursor(0), 0.0);
    assert_eq!(h.fenv_cursor(0), 0.0);
    assert_eq!(h.peak_l(), 0.0);
    assert_eq!(h.peak_r(), 0.0);
    assert_eq!(h.last_latency_us(), 0);
    assert_eq!(h.sample_rate(), 0);
    assert_eq!(h.buffer_frames(), 0);
    // Arp/walker toggles start disabled.
    assert!(!h.arp_enabled());
    assert!(!h.walker_enabled());
}

#[test]
fn descriptor_format_renders_common_cases() {
    // One descriptor from each unit family.
    let cutoff = all_params().iter().find(|d| d.id == ParamId::FilterCutoff).unwrap();
    assert_eq!(cutoff.format(500.0), "500 Hz");
    assert_eq!(cutoff.format(1500.0), "1.50 kHz");

    let attack = all_params().iter().find(|d| d.id == ParamId::AmpAttack).unwrap();
    assert_eq!(attack.format(0.25), "250 ms");
    assert_eq!(attack.format(2.5), "2.50 s");

    let arp_en = all_params().iter().find(|d| d.id == ParamId::ArpEnabled).unwrap();
    assert_eq!(arp_en.format(0.0), "off");
    assert_eq!(arp_en.format(1.0), "on");

    let master = all_params().iter().find(|d| d.id == ParamId::MasterVolume).unwrap();
    assert_eq!(master.format(0.5), "50%");
}

#[test]
fn command_serde_roundtrip() {
    // serde support comes from synth-control's `serde` feature, which is
    // enabled via [dev-dependencies] on synth-engine's Cargo.toml.
    let samples = [
        Command::SetParam { id: ParamId::FilterCutoff, value: 1200.0 },
        Command::NoteOn { pitch: 60, velocity: 100 },
        Command::NoteOff { pitch: 60 },
        Command::AllNotesOff,
        Command::ChordHold(vec![60, 64, 67]),
        Command::ArpRestart,
        Command::WalkerRestart,
    ];
    for cmd in samples {
        let json = serde_json::to_string(&cmd).expect("serialise");
        let back: Command = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(cmd, back, "serde roundtrip mismatch for {:?}", cmd);
    }
}
