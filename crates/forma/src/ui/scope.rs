use crate::SynthApp;
use eframe::egui;
use eframe::egui_wgpu;
use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, Vec2};

use super::scope_wgpu::{HarmParams, ScopeCallback, VizMode, VorParams};

impl SynthApp {
    pub fn ui_oscilloscope(&mut self, ui: &mut egui::Ui) {
        self.draw_scope_panel(ui);
    }

    /// Fullscreen overlay — call once per frame from update() before panels.
    pub fn ui_scope_fullscreen(&mut self, ctx: &egui::Context) {
        if !self.scope_fullscreen {
            return;
        }
        let screen = ctx.screen_rect();
        egui::Window::new("scope_fs_window")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .fixed_pos(screen.min)
            .fixed_size(screen.size())
            .frame(egui::Frame::new().fill(self.theme.c(&self.theme.bg_app)))
            .show(ctx, |ui| {
                self.draw_scope_panel(ui);
            });
    }

    fn draw_scope_panel(&mut self, ui: &mut egui::Ui) {
        let accent = self.theme.c(&self.theme.accent);
        let scope_ctrl = self.theme.c(&self.theme.scope_label);
        let text_sec = self.theme.c(&self.theme.text_secondary);

        // ── Live audio analysis — drives all viz modes ────────────────────────
        let buf = self.engine.scope_buffer_snapshot();
        let rms: f32 = {
            let sum_sq: f32 = buf.iter().map(|&s| s * s).sum();
            (sum_sq / buf.len().max(1) as f32).sqrt().clamp(0.0, 1.0)
        };
        let gates = self.engine.voice_gates();

        // harm_phase drifts slowly when silent, spins when playing
        let dt = ui.ctx().input(|i| i.stable_dt) as f64;
        self.harm_phase += dt * (0.04 + rms as f64 * 1.1);
        self.vor_time += dt;
        ui.ctx().request_repaint();

        // ── Toolbar ────────────────────────────────────────────────────────────
        ui.horizontal(|ui| {
            // Mode picker: SCOPE / HARM / VOR
            for (mode, label, tip) in [
                (VizMode::Scope, "SCOPE", "Waveform oscilloscope"),
                (
                    VizMode::Harmonograph,
                    "HARM",
                    "Harmonograph — parametric pendulum art",
                ),
                (
                    VizMode::Voronoi,
                    "VOR",
                    "Voronoi cells driven by synth parameters",
                ),
            ] {
                let col = if self.viz_mode == mode {
                    accent
                } else {
                    scope_ctrl
                };
                if ui
                    .button(egui::RichText::new(label).small().color(col))
                    .on_hover_text(tip)
                    .clicked()
                {
                    self.viz_mode = mode;
                }
            }

            // X/Y zoom — only meaningful for Scope mode
            if self.viz_mode == VizMode::Scope {
                ui.add_space(4.0);
                ui.label(egui::RichText::new("X").small().color(scope_ctrl))
                    .on_hover_text("Horizontal zoom — stretch or compress the waveform in time.");
                ui.add(
                    egui::DragValue::new(&mut self.scope_x_scale)
                        .speed(0.02)
                        .range(0.25_f32..=8.0)
                        .suffix("×"),
                )
                .on_hover_text("Horizontal zoom (0.25–8×). Drag left/right to adjust.");

                ui.add_space(4.0);
                ui.label(egui::RichText::new("Y").small().color(scope_ctrl))
                    .on_hover_text("Vertical zoom — scale the waveform amplitude.");
                ui.add(
                    egui::DragValue::new(&mut self.scope_y_scale)
                        .speed(0.02)
                        .range(0.25_f32..=8.0)
                        .suffix("×"),
                )
                .on_hover_text("Vertical zoom (0.25–8×). Drag left/right to adjust.");
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let fs_col = if self.scope_fullscreen {
                    accent
                } else {
                    text_sec
                };
                if ui
                    .button(
                        egui::RichText::new(if self.scope_fullscreen {
                            "EXIT FULL"
                        } else {
                            "FULL"
                        })
                        .small()
                        .color(fs_col),
                    )
                    .on_hover_text("Toggle fullscreen scope view.")
                    .clicked()
                {
                    self.scope_fullscreen = !self.scope_fullscreen;
                }

                let v_col = if self.show_voice_debug {
                    accent
                } else {
                    Color32::from_gray(80)
                };
                if ui
                    .button(egui::RichText::new("VOICES").small().color(v_col))
                    .on_hover_text("Per-voice gate and envelope stage inspector.")
                    .clicked()
                {
                    self.show_voice_debug = !self.show_voice_debug;
                }
            });
        });

        // ── Voice inspector ────────────────────────────────────────────────────
        if self.show_voice_debug {
            let cursors = self.engine.amp_cursors();
            ui.horizontal(|ui| {
                for vi in 0..gates.len() {
                    let gate = gates[vi];
                    let cursor = cursors[vi];
                    let stage = match cursor as u8 {
                        0 => "idle",
                        1 => "A",
                        2 => "D",
                        3 => "S",
                        4 => "R",
                        _ => "?",
                    };
                    let (dot_color, label_color) = if gate > 0.5 {
                        (
                            Color32::from_rgb(220, 60, 60),
                            Color32::from_rgb(255, 120, 120),
                        )
                    } else if cursor > 0.5 {
                        (
                            Color32::from_rgb(200, 140, 40),
                            Color32::from_rgb(220, 180, 80),
                        )
                    } else {
                        (Color32::from_gray(50), Color32::from_gray(100))
                    };
                    ui.vertical(|ui| {
                        ui.set_min_width(48.0);
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                            ui.painter().circle_filled(r.center(), 4.0, dot_color);
                            ui.label(
                                egui::RichText::new(format!("V{}", vi + 1))
                                    .small()
                                    .color(label_color),
                            );
                        });
                        ui.label(
                            egui::RichText::new(format!("g:{:.0} {}", gate, stage))
                                .monospace()
                                .size(9.0)
                                .color(label_color),
                        );
                    });
                }
            });
        }

        // ── Canvas area — waveform fills the full available space ────────────
        let avail = ui.available_size();
        let (row_resp, cpu_painter) = ui.allocate_painter(avail, Sense::hover());
        let row = row_resp.rect;

        let ppp = ui.ctx().pixels_per_point();
        let vp_w = (row.width() * ppp).round() as u32;
        let vp_h = (row.height() * ppp).round() as u32;

        // ── Spectrum analysis — 8 log-spaced bins 80 Hz → 8 kHz ─────────────
        let sr = self.engine.sample_rate();
        const N_SPEC: usize = 8;
        let (spec_amp, spec_phase) = compute_spectrum(&buf, sr, N_SPEC);

        // Rank bins by amplitude so we can pick dominant peaks
        let mut peak_order: [usize; N_SPEC] = [0, 1, 2, 3, 4, 5, 6, 7];
        peak_order.sort_unstable_by(|&a, &b| {
            spec_amp[b]
                .partial_cmp(&spec_amp[a])
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // ── Harmonograph — pendulum freqs from spectral peaks ─────────────────
        // spec_weight: 0 at silence (stable OSC-based pattern), 1 when playing
        let spec_weight = (rms * 6.0).clamp(0.0, 1.0);
        let oct_diff = (self.osc_octave[1] - self.osc_octave[0]) as f32;
        let det_diff = (self.osc_detune[1] - self.osc_detune[0]) / 100.0;
        let osc_ratio = 2.0_f32.powf(oct_diff + det_diff).clamp(1.0, 7.0);
        let blend = |a: f32, b: f32| a + (b - a) * spec_weight;
        // Map bin index to integer harmonic for clean Lissajous ratios
        let hf = |bin: usize| (bin + 1) as f32;
        let phase = self.harm_phase as f32;
        let harm_params = HarmParams {
            freqs: [
                blend(1.0, hf(peak_order[0])),
                blend(osc_ratio, hf(peak_order[1])),
                blend(osc_ratio, hf(peak_order[2])),
                blend(1.5, hf(peak_order[3])),
            ],
            // DFT phases inject the actual spectral phase into the pendulum start angles
            phases: [
                blend(phase, spec_phase[peak_order[0]] + phase),
                blend(phase * 0.73 + 0.5, spec_phase[peak_order[1]] + phase * 0.73),
                blend(phase * 1.29, spec_phase[peak_order[2]] + phase * 1.29),
                blend(phase * 0.47 + 1.2, spec_phase[peak_order[3]] + phase * 0.47),
            ],
            damping: (0.08 - rms * 0.065).clamp(0.015, 0.08),
            t_max: 120.0 + rms * 380.0,
            viewport: [vp_w.max(1) as f32, vp_h.max(1) as f32],
        };

        // ── Voronoi — pitch-driven radial clusters ────────────────────────────
        // Seeds radiate from center in concentric rings, one ring per active voice.
        // Pitch → ring radius (bass = far from center, treble = near center).
        // Pitch → seed count on ring (treble = many, bass = few).
        // → dense tessellation near center, sparse large cells at periphery.
        // Envelope amplitude → how far the ring has spread from center.
        // LFO depth → radial wobble of individual seeds.
        // Silence → single seed at center = glowing dot.
        let t = self.vor_time as f32;
        let mut seeds = [[0.0_f32; 4]; 16];
        let mut n_seeds: usize = 0;

        let freqs = self.engine.voice_freqs();
        let cursors = self.engine.amp_cursors();
        // cursor encoding: 0=idle, 1.x=attack (frac=progress), 2.x=decay,
        // 3.0=sustain (exact integer — fract() would give 0!), 4.x=release
        let env_amp = |cursor: f32| -> f32 {
            match cursor as u8 {
                0 => 0.0,
                1 => cursor.fract(),                  // attack: 0 → 1
                2 => 1.0 - cursor.fract() * 0.15,     // decay: 1 → 0.85
                3 => 0.85,                            // sustain: held
                4 => (1.0 - cursor.fract()).max(0.0), // release: 1 → 0
                _ => 0.0,
            }
        };
        let lfo_wobble = (self.lfo_depth * 0.14 + self.lfo2_depth * 0.09).clamp(0.0, 0.28);

        for vi in 0..freqs.len() {
            let freq = freqs[vi];
            let cursor = cursors[vi];
            let amp = env_amp(cursor);
            if freq < 20.0 || amp < 0.02 {
                continue;
            }

            // pitch_t: 0 = A0 (27.5 Hz) = bass, 1 = A8 (7040 Hz) = treble (8 octaves)
            let pitch_t = ((freq / 27.5).log2() / 8.0).clamp(0.0, 1.0);

            // Bass → far ring, treble → near ring; all scale with envelope amplitude
            let max_r = 0.10 + (1.0 - pitch_t) * 0.34; // bass 0.44, treble 0.10
            let ring_r = max_r * amp;

            // Treble = many seeds (dense small cells), bass = few (large cells)
            let n_voice = (3.0 + pitch_t * 9.0).round() as usize; // 3 – 12

            // Angular offset per voice so rings from different voices don't fully overlap
            let angle_off = vi as f32 * std::f32::consts::TAU / freqs.len() as f32 * 0.37;

            for si in 0..n_voice {
                if n_seeds >= 16 {
                    break;
                }
                let base_angle = si as f32 * std::f32::consts::TAU / n_voice as f32 + angle_off;
                let wobble_r = lfo_wobble
                    * ring_r
                    * (self.lfo_rate * t * 0.4 + base_angle + vi as f32 * 1.3).sin();
                let r = (ring_r + wobble_r).max(0.0);
                seeds[n_seeds][0] = (0.5 + r * base_angle.cos()).clamp(0.02, 0.98);
                seeds[n_seeds][1] = (0.5 + r * base_angle.sin()).clamp(0.02, 0.98);
                n_seeds += 1;
            }
        }

        if n_seeds == 0 {
            seeds[0] = [0.5, 0.5, 0.0, 0.0];
            n_seeds = 1;
        }
        let vor_params = VorParams {
            seeds,
            num_seeds: n_seeds as u32,
            beat_pulse: rms,
            tex_w: vp_w.max(1) as f32,
            tex_h: vp_h.max(1) as f32,
        };

        cpu_painter.add(egui_wgpu::Callback::new_paint_callback(
            row,
            ScopeCallback {
                samples: buf,
                x_scale: self.scope_x_scale,
                y_scale: self.scope_y_scale,
                viewport_size: (vp_w.max(1), vp_h.max(1)),
                viz_mode: self.viz_mode,
                harm_params,
                vor_params,
            },
        ));
    }
}

/// Compute DFT magnitude and phase at `n_bins` logarithmically-spaced frequencies
/// from 80 Hz to 8 kHz.  Uses up to 1024 samples for a good speed/resolution trade-off.
/// Magnitudes are normalised to [0, 1] relative to the strongest bin, then sqrt-compressed
/// for a perceptual scale where quiet bands still register.  Phases are in [-π, π].
fn compute_spectrum(buf: &[f32], sample_rate: u32, n_bins: usize) -> (Vec<f32>, Vec<f32>) {
    use std::f32::consts::PI;
    let sr = if sample_rate > 0 {
        sample_rate as f32
    } else {
        44100.0
    };
    let f_min = 80.0_f32;
    let f_max = 8000.0_f32;
    let n = buf.len().min(1024);
    if n < 4 || n_bins == 0 {
        return (vec![0.0; n_bins], vec![0.0; n_bins]);
    }
    let mut amps = vec![0.0_f32; n_bins];
    let mut phases = vec![0.0_f32; n_bins];
    for k in 0..n_bins {
        let t = k as f32 / (n_bins - 1).max(1) as f32;
        let freq = f_min * (f_max / f_min).powf(t); // log spacing
        let step = 2.0 * PI * freq / sr;
        let (mut re, mut im) = (0.0_f32, 0.0_f32);
        for (i, &s) in buf[..n].iter().enumerate() {
            let a = step * i as f32;
            re += s * a.cos();
            im += s * a.sin();
        }
        amps[k] = (re * re + im * im).sqrt() / n as f32;
        phases[k] = im.atan2(re);
    }
    // Normalise shape to [0,1]; multiply by RMS at call site to scale with energy
    let peak = amps.iter().cloned().fold(0.0_f32, f32::max).max(1e-7);
    for a in &mut amps {
        *a = (*a / peak).powf(0.55); // sqrt-ish perceptual compression
    }
    (amps, phases)
}

pub fn draw_latency_bar(
    ui: &mut egui::Ui,
    engine: &forma_engine::SynthEngineHandle,
    attack_s: f32,
    theme: &super::theme::SynthTheme,
) {
    let sr = engine.sample_rate();
    let frames = engine.buffer_frames();
    let measured_us = engine.last_latency_us();

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Latency:").weak().small());

        if sr == 0 || frames == 0 {
            ui.label(egui::RichText::new("measuring…").weak().small().italics());
            return;
        }

        let buffer_ms = frames as f32 / sr as f32 * 1000.0;
        let ui_ms = 1000.0 / 60.0;
        let attack_ms = attack_s * 1000.0;
        let est_ms = buffer_ms + ui_ms + attack_ms;

        let est_color = if est_ms < 20.0 {
            theme.c(&theme.latency_ok)
        } else if est_ms < 40.0 {
            theme.c(&theme.latency_warn)
        } else {
            theme.c(&theme.latency_bad)
        };
        ui.label(
            egui::RichText::new(format!(
                "est ~{est_ms:.0}ms  (buf {buffer_ms:.1} + UI ~{ui_ms:.0} + atk {attack_ms:.0})"
            ))
            .small()
            .color(est_color),
        );

        if measured_us > 0 {
            let measured_ms = measured_us as f32 / 1000.0;
            let meas_color = if measured_ms < 20.0 {
                theme.c(&theme.accent)
            } else if measured_ms < 40.0 {
                theme.c(&theme.latency_warn)
            } else {
                theme.c(&theme.latency_bad)
            };
            ui.separator();
            ui.label(
                egui::RichText::new(format!("measured {measured_ms:.1}ms"))
                    .small()
                    .strong()
                    .color(meas_color),
            );
        }
    });
}

pub fn draw_peak_meter(
    ui: &mut egui::Ui,
    level: f32,
    peak_hold: f32,
    theme: &super::theme::SynthTheme,
) {
    let (resp, painter) =
        ui.allocate_painter(Vec2::new(ui.available_width(), 14.0), Sense::hover());
    let rect = resp.rect;
    painter.rect_filled(rect, CornerRadius::same(2), theme.c(&theme.meter_bg));

    let max_display = 1.5_f32;
    let bar_frac = (level / max_display).clamp(0.0, 1.0);
    let bar_w = rect.width() * bar_frac;

    if bar_w > 0.5 {
        let color = if level < 0.7 {
            theme.c(&theme.meter_green)
        } else if level < 1.0 {
            let t = (level - 0.7) / 0.3;
            let g = theme.meter_green;
            let c = theme.meter_clip;
            Color32::from_rgb(
                (g[0] as f32 + (c[0] as f32 - g[0] as f32) * t) as u8,
                (g[1] as f32 + (c[1] as f32 - g[1] as f32) * t) as u8,
                (g[2] as f32 + (c[2] as f32 - g[2] as f32) * t) as u8,
            )
        } else {
            theme.c(&theme.meter_clip)
        };
        let bar_rect = Rect::from_min_size(rect.min, Vec2::new(bar_w, rect.height()));
        painter.rect_filled(bar_rect, CornerRadius::same(2), color);
    }

    let unity_x = rect.left() + rect.width() * (1.0 / max_display);
    painter.line_segment(
        [
            Pos2::new(unity_x, rect.top()),
            Pos2::new(unity_x, rect.bottom()),
        ],
        Stroke::new(1.0, Color32::from_rgba_premultiplied(255, 255, 255, 100)),
    );

    if peak_hold > 0.01 {
        let hold_frac = (peak_hold / max_display).clamp(0.0, 1.0);
        let hold_x = rect.left() + rect.width() * hold_frac;
        let hold_color = if peak_hold >= 1.0 {
            theme.c(&theme.meter_clip)
        } else {
            Color32::WHITE
        };
        painter.line_segment(
            [
                Pos2::new(hold_x, rect.top() + 1.0),
                Pos2::new(hold_x, rect.bottom() - 1.0),
            ],
            Stroke::new(2.0, hold_color),
        );
    }

    let text = if level >= 1.0 {
        format!("{:+.1} dB CLIP", 20.0 * level.log10())
    } else if level > 0.001 {
        format!("{:+.1} dB", 20.0 * level.log10())
    } else {
        "-inf dB".to_string()
    };
    painter.text(
        Pos2::new(rect.left() + 4.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(10.0),
        Color32::WHITE,
    );
}
