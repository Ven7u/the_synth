//! Headless smoke test for `SynthEngineHandle` + protocol.
//!
//! No cpal, no eframe. Builds the handle directly from an `AudioState` and
//! a control channel, exercises every equivalence path we care about, and
//! asserts descriptor-table invariants.

use std::collections::HashSet;
use std::sync::Arc;

use forma_control::{all_params, make_control_channel, Command, ControlEvent, ParamId, ParamKind};
use forma_engine::{AudioState, Patch, SynthEngineHandle};

fn make_handle() -> (SynthEngineHandle, forma_control::ControlReceiver) {
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
    h.apply(Command::SetParam {
        id: ParamId::FilterCutoff,
        value: 3000.0,
    });
    assert!((h.filter_cutoff() - 3000.0).abs() < 1e-3);

    h.apply(Command::SetParam {
        id: ParamId::LfoShape,
        value: 1.0,
    });
    assert_eq!(h.lfo_shape(), 1);

    h.apply(Command::SetParam {
        id: ParamId::LimiterEnabled,
        value: 0.0,
    });
    assert!(!h.limiter_enabled());
}

#[test]
fn events_land_on_the_channel() {
    let (h, rx) = make_handle();

    h.note_on(60, 100);
    let ev = rx.try_recv().expect("NoteOn should be on the channel");
    match ev {
        ControlEvent::NoteOn {
            pitch,
            velocity,
            track,
        } => {
            assert_eq!(pitch, 60);
            assert_eq!(velocity, 100);
            assert_eq!(track, 0);
        }
        other => panic!("unexpected event: {other:?}"),
    }

    h.note_off(60);
    let ev = rx.try_recv().unwrap();
    assert!(matches!(
        ev,
        ControlEvent::NoteOff {
            pitch: 60,
            track: 0
        }
    ));

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
            desc.id,
            desc.min,
            desc.default
        );
        assert!(
            desc.default <= desc.max,
            "{:?}: default {} > max {}",
            desc.id,
            desc.default,
            desc.max
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
        h.apply(Command::SetParam {
            id: desc.id,
            value: desc.default,
        });

        // For params we can also read back, confirm semantics.
        if let Some(got) = h.get_by_id(desc.id) {
            match desc.kind {
                ParamKind::Bool => {
                    let want = if desc.default != 0.0 { 1.0 } else { 0.0 };
                    assert!(
                        (got - want).abs() < 1e-3,
                        "{:?}: bool round-trip failed — want {}, got {}",
                        desc.id,
                        want,
                        got
                    );
                }
                ParamKind::Discrete(_) => {
                    let want = desc.default.round();
                    assert!(
                        (got - want).abs() < 1e-3,
                        "{:?}: discrete round-trip failed — want {}, got {}",
                        desc.id,
                        want,
                        got
                    );
                }
                ParamKind::Linear | ParamKind::Log => {
                    assert!(
                        (got - desc.default).abs() < 1e-3
                            || (got - desc.default).abs() / desc.default.abs().max(1e-6) < 1e-3,
                        "{:?}: numeric round-trip failed — want {}, got {}",
                        desc.id,
                        desc.default,
                        got
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
    let cutoff = all_params()
        .iter()
        .find(|d| d.id == ParamId::FilterCutoff)
        .unwrap();
    assert_eq!(cutoff.format(500.0), "500 Hz");
    assert_eq!(cutoff.format(1500.0), "1.50 kHz");

    let attack = all_params()
        .iter()
        .find(|d| d.id == ParamId::AmpAttack)
        .unwrap();
    assert_eq!(attack.format(0.25), "250 ms");
    assert_eq!(attack.format(2.5), "2.50 s");

    let arp_en = all_params()
        .iter()
        .find(|d| d.id == ParamId::ArpEnabled)
        .unwrap();
    assert_eq!(arp_en.format(0.0), "off");
    assert_eq!(arp_en.format(1.0), "on");

    let master = all_params()
        .iter()
        .find(|d| d.id == ParamId::MasterVolume)
        .unwrap();
    assert_eq!(master.format(0.5), "50%");
}

#[test]
fn command_serde_roundtrip() {
    // serde support comes from forma-control's `serde` feature, which is
    // enabled via [dev-dependencies] on forma-engine's Cargo.toml.
    let samples = [
        Command::SetParam {
            id: ParamId::FilterCutoff,
            value: 1200.0,
        },
        Command::NoteOn {
            pitch: 60,
            velocity: 100,
        },
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

#[test]
fn apply_patch_writes_engine_state() {
    // Apply a custom patch through the handle and verify the live values land.
    let (h, _rx) = make_handle();
    let mut p = h.snapshot_patch(); // start from current defaults

    p.name = "Test".into();
    p.category = "Unit".into();
    p.osc_wave = [2, 3, 0];
    p.osc_octave = [1, -1, 0];
    p.osc_detune = [0.0, 0.0, 0.0];
    p.osc_vol = [0.55, 0.25, 0.1];
    p.osc_enabled = [true, true, false]; // osc 3 muted → vol 0
    p.filter_enabled = true;
    p.filter_cutoff = 1234.0;
    p.filter_q = 0.65;
    p.filter_env_amount = 0.5;
    p.fenv_adsr = [0.01, 0.2, 0.4, 0.3];
    p.amp_adsr = [0.05, 0.25, 0.7, 0.4];
    p.lfo_enabled = true;
    p.lfo_rate = 3.0;
    p.lfo_depth = 0.5;
    p.lfo_shape = 1;
    p.lfo_dest = 2;
    p.glide_time = 0.12;
    p.master_vol = 0.7;
    p.global_vol = 0.85;
    p.limiter_enabled = false;
    p.limiter_threshold = 0.6;
    p.fx_delay_on = true;
    p.fx_delay_time = 0.28;
    p.fx_delay_feedback = 0.55;
    p.fx_delay_mix = 0.35;
    p.fx_reverb_on = true;
    p.fx_reverb_mix = 0.4;
    p.fx_shimmer_on = true;
    p.fx_shimmer_mix = 0.3;
    p.fx_shimmer_amt = 0.6;

    h.apply_patch(&p);

    // Oscillator bank — osc 3 was disabled, engine vol should be 0.
    assert_eq!(h.osc_wave(0), 2);
    assert_eq!(h.osc_wave(1), 3);
    assert_eq!(h.osc_wave(2), 0);
    assert!((h.osc_vol(0) - 0.55).abs() < 1e-4);
    assert!((h.osc_vol(1) - 0.25).abs() < 1e-4);
    assert_eq!(h.osc_vol(2), 0.0, "disabled osc should have engine vol 0");

    // Filter + env
    assert!((h.filter_cutoff() - 1234.0).abs() < 1e-3);
    assert!((h.filter_resonance() - 0.65).abs() < 1e-4);
    assert!((h.filter_env_amount() - 0.5).abs() < 1e-4);
    assert!((h.fenv_attack() - 0.01).abs() < 1e-5);
    assert!((h.amp_sustain() - 0.7).abs() < 1e-4);

    // LFO
    assert!((h.lfo_rate() - 3.0).abs() < 1e-4);
    assert!((h.lfo_depth() - 0.5).abs() < 1e-4);
    assert_eq!(h.lfo_shape(), 1);
    assert_eq!(h.lfo_dest(), 2);

    // Master / limiter
    assert!((h.master_volume() - 0.7).abs() < 1e-4);
    assert!((h.global_volume() - 0.85).abs() < 1e-4);
    assert!(!h.limiter_enabled());
    assert!((h.limiter_threshold() - 0.6).abs() < 1e-4);

    // FX — shimmer has an on flag; disabled should mute both amt and mix.
    assert!((h.fx_delay_time() - 0.28).abs() < 1e-4);
    assert!((h.fx_delay_mix() - 0.35).abs() < 1e-4);
    assert!((h.fx_reverb_mix() - 0.4).abs() < 1e-4);
    assert!((h.shimmer_mix() - 0.3).abs() < 1e-4);
    assert!((h.shimmer_amount() - 0.6).abs() < 1e-4);
}

#[test]
fn snapshot_then_apply_is_a_fixed_point() {
    // snapshot(engine) → apply(engine) should leave engine state unchanged.
    let (h, _rx) = make_handle();

    // Nudge a few engine params so we're not round-tripping defaults.
    h.set_filter_cutoff(2200.0);
    h.set_master_volume(0.55);
    h.set_fx_delay_mix(0.3);
    h.set_fx_delay_time(0.4);
    h.set_shimmer_mix(0.25);

    let snap = h.snapshot_patch();
    h.apply_patch(&snap);

    assert!((h.filter_cutoff() - 2200.0).abs() < 1e-3);
    assert!((h.master_volume() - 0.55).abs() < 1e-4);
    assert!((h.fx_delay_mix() - 0.3).abs() < 1e-4);
    assert!((h.fx_delay_time() - 0.4).abs() < 1e-4);
    assert!((h.shimmer_mix() - 0.25).abs() < 1e-4);
}

#[test]
fn patch_serde_roundtrip() {
    // Patch must survive JSON serialisation so existing assets/patches/*.json
    // keep loading after the crate move.
    let (h, _rx) = make_handle();
    let mut p = h.snapshot_patch();
    p.name = "Round".into();
    p.category = "Trip".into();

    let json = serde_json::to_string(&p).expect("serialise");
    let back: Patch = serde_json::from_str(&json).expect("deserialise");

    assert_eq!(back.name, "Round");
    assert_eq!(back.category, "Trip");
    assert_eq!(back.osc_wave, p.osc_wave);
    assert!((back.filter_cutoff - p.filter_cutoff).abs() < 1e-5);
    assert_eq!(back.lfo_shape, p.lfo_shape);
    assert!((back.master_vol - p.master_vol).abs() < 1e-5);
}
