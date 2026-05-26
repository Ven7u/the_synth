use crate::SynthApp;
use eframe::egui;
use eframe::egui_wgpu;
use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, Vec2};

use super::scope_wgpu::{HarmParams, ScopeCallback, VizMode, VorParams, SGR_ROWS};

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
            // Mode picker: SCOPE / HARM / VOR / SPEC / SGR / ENV
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
                (
                    VizMode::Spectrum,
                    "SPEC",
                    "Spectrum analyzer — frequency content 20 Hz – 20 kHz",
                ),
                (
                    VizMode::Spectrogram,
                    "SGR",
                    "Spectrogram — time scrolls left→right, frequency on Y",
                ),
                (
                    VizMode::SpectrogramV,
                    "SGRV",
                    "Spectrogram — frequency on X, time scrolls bottom→top",
                ),
                (
                    VizMode::Envelope,
                    "ENV",
                    "Envelope visualizer — live ADSR curve with voice cursors",
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

        // ── CPU-drawn modes: Spectrum and Envelope ────────────────────────────
        if self.viz_mode == VizMode::Spectrum {
            draw_spectrum(&cpu_painter, row, &buf, self.engine.sample_rate());
            return;
        }
        if self.viz_mode == VizMode::Envelope {
            let a = self.engine.amp_attack();
            let d = self.engine.amp_decay();
            let s = self.engine.amp_sustain();
            let r = self.engine.amp_release();
            let cursors = self.engine.amp_cursors();
            draw_envelope(&cpu_painter, row, a, d, s, r, &cursors);
            return;
        }

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

        // Compute spectrogram bins on CPU (GPU handles ring buffer + colormap).
        let sgr_bins = if matches!(self.viz_mode, VizMode::Spectrogram | VizMode::SpectrogramV) {
            Some(compute_sgr_bins(&buf, self.engine.sample_rate()))
        } else {
            None
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
                sgr_bins,
            },
        ));

        // Axis labels drawn on top of the GPU render.
        match self.viz_mode {
            VizMode::Spectrogram => draw_sgr_labels(&cpu_painter, row),
            VizMode::SpectrogramV => draw_sgrv_labels(&cpu_painter, row),
            _ => {}
        }
    }
}

// Phosphor CRT palette — matches the wgpu beam colours (0.13, 1.0, 0.55).
const PHOSPHOR: Color32 = Color32::from_rgb(33, 255, 140);
const PHOSPHOR_MID: Color32 = Color32::from_rgb(26, 166, 107);
const PHOSPHOR_DIM: Color32 = Color32::from_rgb(15, 80, 55);
const PHOSPHOR_BG: Color32 = Color32::from_rgb(4, 12, 9);

const F_MIN_HZ: f32 = 30.0;
const F_MAX_HZ: f32 = 20_000.0;

/// Compute SGR_ROWS log-spaced Hann-windowed magnitude bins for one spectrogram column.
fn compute_sgr_bins(buf: &[f32], sample_rate: u32) -> Vec<f32> {
    use std::f32::consts::PI;
    let sr = if sample_rate > 0 {
        sample_rate as f32
    } else {
        44100.0
    };
    let n = buf.len().min(2048);
    const DB_FLOOR: f32 = -70.0;
    let rows = SGR_ROWS as usize;
    if n < 8 {
        return vec![0.0; rows];
    }
    let mut col = Vec::with_capacity(rows);
    for k in 0..rows {
        let t = k as f32 / (rows - 1) as f32;
        let freq = F_MIN_HZ * (F_MAX_HZ / F_MIN_HZ).powf(t);
        let step = 2.0 * PI * freq / sr;
        let (mut re, mut im, mut w_sum) = (0.0f32, 0.0f32, 0.0f32);
        for (i, &s) in buf[..n].iter().enumerate() {
            let w = 0.5 * (1.0 - (2.0 * PI * i as f32 / (n - 1) as f32).cos());
            re += s * w * (step * i as f32).cos();
            im += s * w * (step * i as f32).sin();
            w_sum += w;
        }
        let mag = (re * re + im * im).sqrt() / w_sum.max(1e-9);
        let db = 20.0 * mag.max(1e-9).log10();
        col.push(((db - DB_FLOOR) / (-DB_FLOOR)).clamp(0.0, 1.0));
    }
    col
}

/// Draw frequency axis labels on top of the GPU-rendered spectrogram.
/// Padding values must match the shader's pad_l/pad_b/pad_t/pad_r constants.
fn draw_sgr_labels(painter: &egui::Painter, rect: Rect) {
    let pad_l = 38.0f32;
    let pad_b = 18.0f32;
    let pad_t = 6.0f32;
    let pad_r = 6.0f32;
    let draw_rect = Rect::from_min_max(
        Pos2::new(rect.left() + pad_l, rect.top() + pad_t),
        Pos2::new(rect.right() - pad_r, rect.bottom() - pad_b),
    );
    let h = draw_rect.height();

    for &(freq_hz, label) in &[
        (100.0f32, "100"),
        (500.0, "500"),
        (1000.0, "1k"),
        (2000.0, "2k"),
        (5000.0, "5k"),
        (10000.0, "10k"),
        (20000.0, "20k"),
    ] {
        let t = ((freq_hz / F_MIN_HZ).log2() / (F_MAX_HZ / F_MIN_HZ).log2()).clamp(0.0, 1.0);
        let y = draw_rect.bottom() - t * h;
        painter.line_segment(
            [
                Pos2::new(draw_rect.left(), y),
                Pos2::new(draw_rect.right(), y),
            ],
            Stroke::new(0.5, Color32::from_rgba_premultiplied(15, 55, 38, 100)),
        );
        painter.text(
            Pos2::new(rect.left() + 2.0, y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::monospace(8.0),
            PHOSPHOR_MID,
        );
    }

    painter.text(
        Pos2::new(draw_rect.center_bottom().x, rect.bottom() - 2.0),
        egui::Align2::CENTER_BOTTOM,
        "time →",
        egui::FontId::monospace(8.0),
        PHOSPHOR_DIM,
    );

    // "kHz" vertical label on left
    painter.text(
        Pos2::new(rect.left() + 2.0, draw_rect.center().y),
        egui::Align2::LEFT_CENTER,
        "kHz",
        egui::FontId::monospace(8.0),
        PHOSPHOR_DIM,
    );
}

/// Axis labels for SGRV mode: frequency on X (bottom), time on Y (left = "now" at top).
fn draw_sgrv_labels(painter: &egui::Painter, rect: Rect) {
    let pad_l = 38.0f32;
    let pad_b = 18.0f32;
    let pad_t = 6.0f32;
    let pad_r = 6.0f32;
    let draw_rect = Rect::from_min_max(
        Pos2::new(rect.left() + pad_l, rect.top() + pad_t),
        Pos2::new(rect.right() - pad_r, rect.bottom() - pad_b),
    );
    let w = draw_rect.width();
    let h = draw_rect.height();

    // Frequency axis labels along the bottom (X = frequency).
    for &(freq_hz, label) in &[
        (100.0f32, "100"),
        (500.0, "500"),
        (1000.0, "1k"),
        (2000.0, "2k"),
        (5000.0, "5k"),
        (10000.0, "10k"),
        (20000.0, "20k"),
    ] {
        let t = ((freq_hz / F_MIN_HZ).log2() / (F_MAX_HZ / F_MIN_HZ).log2()).clamp(0.0, 1.0);
        let x = draw_rect.left() + t * w;
        painter.line_segment(
            [
                Pos2::new(x, draw_rect.top()),
                Pos2::new(x, draw_rect.bottom()),
            ],
            Stroke::new(0.5, Color32::from_rgba_premultiplied(15, 55, 38, 100)),
        );
        painter.text(
            Pos2::new(x, rect.bottom() - 2.0),
            egui::Align2::CENTER_BOTTOM,
            label,
            egui::FontId::monospace(8.0),
            PHOSPHOR_MID,
        );
    }

    // Time axis labels on the left (Y = time, bottom=oldest, top=newest).
    for &(frac, label) in &[(0.0f32, "now"), (0.5, ""), (1.0, "old")] {
        let y = draw_rect.top() + frac * h;
        painter.line_segment(
            [
                Pos2::new(draw_rect.left(), y),
                Pos2::new(draw_rect.right(), y),
            ],
            Stroke::new(0.5, Color32::from_rgba_premultiplied(15, 55, 38, 60)),
        );
        if !label.is_empty() {
            painter.text(
                Pos2::new(rect.left() + 2.0, y),
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::monospace(8.0),
                PHOSPHOR_DIM,
            );
        }
    }

    // Axis title labels.
    painter.text(
        Pos2::new(draw_rect.center_bottom().x, rect.bottom() - 2.0),
        egui::Align2::CENTER_BOTTOM,
        "Hz",
        egui::FontId::monospace(8.0),
        PHOSPHOR_DIM,
    );
    painter.text(
        Pos2::new(rect.left() + 2.0, draw_rect.center().y),
        egui::Align2::LEFT_CENTER,
        "t↑",
        egui::FontId::monospace(8.0),
        PHOSPHOR_DIM,
    );
}

/// 80 log-spaced bins 20 Hz → 20 kHz, Hann windowed, dB-scaled.
fn draw_spectrum(painter: &egui::Painter, rect: Rect, buf: &[f32], sample_rate: u32) {
    use std::f32::consts::PI;

    painter.rect_filled(rect, CornerRadius::same(4), PHOSPHOR_BG);

    let sr = if sample_rate > 0 {
        sample_rate as f32
    } else {
        44100.0
    };
    let n = buf.len().min(2048);
    if n < 8 {
        return;
    }

    const N_BINS: usize = 80;
    const F_MIN: f32 = 20.0;
    const F_MAX: f32 = 20_000.0;
    const DB_FLOOR: f32 = -70.0;

    // Hann-windowed DFT at each log-spaced frequency.
    let mut bins = [0.0f32; N_BINS];
    for k in 0..N_BINS {
        let t = k as f32 / (N_BINS - 1) as f32;
        let freq = F_MIN * (F_MAX / F_MIN).powf(t);
        let step = 2.0 * PI * freq / sr;
        let (mut re, mut im) = (0.0f32, 0.0f32);
        let mut w_sum = 0.0f32;
        for (i, &s) in buf[..n].iter().enumerate() {
            let w = 0.5 * (1.0 - (2.0 * PI * i as f32 / (n - 1) as f32).cos());
            re += s * w * (step * i as f32).cos();
            im += s * w * (step * i as f32).sin();
            w_sum += w;
        }
        let mag = (re * re + im * im).sqrt() / w_sum.max(1e-9);
        let db = 20.0 * mag.max(1e-9).log10();
        bins[k] = ((db - DB_FLOOR) / (-DB_FLOOR)).clamp(0.0, 1.0);
    }

    let pad_l = 36.0;
    let pad_b = 18.0;
    let pad_t = 8.0;
    let draw_w = rect.width() - pad_l - 4.0;
    let draw_h = rect.height() - pad_b - pad_t;
    let draw_origin = Pos2::new(rect.left() + pad_l, rect.top() + pad_t);

    // dB grid lines and labels — dim phosphor green.
    for &db_mark in &[-60.0f32, -48.0, -36.0, -24.0, -12.0, 0.0] {
        let y_t = ((db_mark - DB_FLOOR) / (-DB_FLOOR)).clamp(0.0, 1.0);
        let y = draw_origin.y + draw_h * (1.0 - y_t);
        painter.line_segment(
            [
                Pos2::new(draw_origin.x, y),
                Pos2::new(draw_origin.x + draw_w, y),
            ],
            Stroke::new(0.5, Color32::from_rgba_premultiplied(15, 60, 40, 140)),
        );
        painter.text(
            Pos2::new(rect.left() + 2.0, y),
            egui::Align2::LEFT_CENTER,
            format!("{db_mark:.0}"),
            egui::FontId::monospace(8.0),
            PHOSPHOR_MID,
        );
    }

    // Frequency axis labels.
    for &(freq_hz, label) in &[
        (50.0f32, "50"),
        (100.0, "100"),
        (200.0, "200"),
        (500.0, "500"),
        (1000.0, "1k"),
        (2000.0, "2k"),
        (5000.0, "5k"),
        (10000.0, "10k"),
        (20000.0, "20k"),
    ] {
        let t = ((freq_hz / F_MIN).log2() / (F_MAX / F_MIN).log2()).clamp(0.0, 1.0);
        let x = draw_origin.x + t * draw_w;
        let y_bot = draw_origin.y + draw_h;
        painter.line_segment(
            [Pos2::new(x, draw_origin.y), Pos2::new(x, y_bot)],
            Stroke::new(0.5, Color32::from_rgba_premultiplied(15, 60, 40, 100)),
        );
        painter.text(
            Pos2::new(x, y_bot + 2.0),
            egui::Align2::CENTER_TOP,
            label,
            egui::FontId::monospace(8.0),
            PHOSPHOR_DIM,
        );
    }

    // Phosphor bars: green (bass) → cyan-mint (treble); amplitude drives brightness.
    let bar_w = (draw_w / N_BINS as f32 - 1.0).max(1.0);
    for (k, &amp) in bins.iter().enumerate() {
        if amp < 0.001 {
            continue;
        }
        let t = k as f32 / (N_BINS - 1) as f32;
        // Frequency hue: warm green at bass, cool mint at treble
        let base_r = 20.0 + t * 20.0; // 20 → 40
        let base_g = 210.0 + t * 30.0; // 210 → 240
        let base_b = 90.0 + t * 100.0; // 90 → 190
                                       // Amplitude drives brightness (sqrt for perceptual linearity)
        let bright = amp.sqrt();
        let bar_color = Color32::from_rgb(
            (base_r * bright) as u8,
            (base_g * bright) as u8,
            (base_b * bright) as u8,
        );

        let x = draw_origin.x + t * draw_w;
        let bar_h = amp * draw_h;
        let y_top = draw_origin.y + draw_h - bar_h;

        // Soft glow behind bar
        painter.rect_filled(
            Rect::from_min_size(Pos2::new(x - 1.0, y_top), Vec2::new(bar_w + 2.0, bar_h)),
            CornerRadius::same(2),
            Color32::from_rgba_premultiplied(
                (base_r * bright * 0.4) as u8,
                (base_g * bright * 0.4) as u8,
                (base_b * bright * 0.4) as u8,
                80,
            ),
        );
        // Main bar
        painter.rect_filled(
            Rect::from_min_size(Pos2::new(x, y_top), Vec2::new(bar_w, bar_h)),
            CornerRadius::same(1),
            bar_color,
        );
        // Bright phosphor cap
        let cap_bright = (bright * 1.4).min(1.0);
        painter.rect_filled(
            Rect::from_min_size(Pos2::new(x, y_top), Vec2::new(bar_w, 2.0)),
            CornerRadius::ZERO,
            Color32::from_rgb(
                (base_r * cap_bright + 30.0 * cap_bright) as u8,
                ((base_g * cap_bright).min(255.0)) as u8,
                (base_b * cap_bright + 20.0 * cap_bright) as u8,
            ),
        );
    }
}

/// Draw the ADSR envelope curve with live voice cursor dots.
fn draw_envelope(
    painter: &egui::Painter,
    rect: Rect,
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    cursors: &[f32],
) {
    use std::f32::consts::PI;
    painter.rect_filled(rect, CornerRadius::same(4), PHOSPHOR_BG);

    let pad_l = 34.0;
    let pad_b = 18.0;
    let pad_t = 10.0;
    let pad_r = 10.0;
    let draw_rect = Rect::from_min_max(
        Pos2::new(rect.left() + pad_l, rect.top() + pad_t),
        Pos2::new(rect.right() - pad_r, rect.bottom() - pad_b),
    );
    let w = draw_rect.width();
    let h = draw_rect.height();

    // Time segments: attack, decay, sustain hold (fixed display), release.
    let s_hold = 0.35;
    let total = (attack + decay + s_hold + release).max(0.001);
    let t_a = attack / total;
    let t_d = decay / total;
    let t_s = s_hold / total;
    let t_r = release / total;

    let x0 = draw_rect.left();
    let x1 = x0 + t_a * w;
    let x2 = x1 + t_d * w;
    let x3 = x2 + t_s * w;
    let x4 = x3 + t_r * w;
    let y_top = draw_rect.top();
    let y_bot = draw_rect.bottom();
    let y_sus = y_top + (1.0 - sustain.clamp(0.0, 1.0)) * h;

    // Amplitude grid lines (0%, 25%, 50%, 75%, 100%).
    for i in 0..=4 {
        let frac = i as f32 / 4.0;
        let y = y_top + (1.0 - frac) * h;
        let label = format!("{:.0}%", frac * 100.0);
        painter.line_segment(
            [Pos2::new(x0, y), Pos2::new(x4, y)],
            Stroke::new(0.5, Color32::from_rgba_premultiplied(15, 55, 38, 120)),
        );
        painter.text(
            Pos2::new(x0 - 3.0, y),
            egui::Align2::RIGHT_CENTER,
            label,
            egui::FontId::monospace(8.0),
            PHOSPHOR_DIM,
        );
    }

    // Stage separator ticks.
    for &(x, label) in &[(x1, "A"), (x2, "D"), (x3, "S"), (x4, "R")] {
        painter.line_segment(
            [Pos2::new(x, y_top), Pos2::new(x, y_bot + 4.0)],
            Stroke::new(0.5, Color32::from_rgba_premultiplied(15, 55, 38, 90)),
        );
        painter.text(
            Pos2::new(x, y_bot + 5.0),
            egui::Align2::CENTER_TOP,
            label,
            egui::FontId::monospace(9.0),
            PHOSPHOR_MID,
        );
    }

    // Envelope outline points: start → attack peak → after decay → sustain hold → release end.
    let env_pts = vec![
        Pos2::new(x0, y_bot),
        Pos2::new(x1, y_top),
        Pos2::new(x2, y_sus),
        Pos2::new(x3, y_sus),
        Pos2::new(x4, y_bot),
    ];

    // Filled area: close polygon along the bottom edge.
    let mut fill_pts = env_pts.clone();
    fill_pts.push(Pos2::new(x0, y_bot));
    painter.add(egui::Shape::Path(egui::epaint::PathShape {
        points: fill_pts,
        closed: true,
        fill: Color32::from_rgba_premultiplied(15, 80, 50, 45),
        stroke: egui::epaint::PathStroke::NONE,
    }));

    // Phosphor glow layer — wider, dimmer stroke.
    painter.add(egui::Shape::line(
        env_pts.clone(),
        Stroke::new(5.0, Color32::from_rgba_premultiplied(33, 255, 140, 25)),
    ));
    // Main bright curve.
    painter.add(egui::Shape::line(env_pts, Stroke::new(1.5, PHOSPHOR)));

    // Live voice cursor dots — stage-colored for clarity.
    for &cursor in cursors {
        let stage = cursor as u8;
        if stage == 0 {
            continue;
        }
        let frac = cursor.fract();

        let (cx, cy) = match stage {
            1 => {
                let x = x0 + frac * (x1 - x0);
                let y = y_bot + frac * (y_top - y_bot);
                (x, y)
            }
            2 => {
                let x = x1 + frac * (x2 - x1);
                let y = y_top + frac * (y_sus - y_top);
                (x, y)
            }
            3 => {
                let pulse = (frac * PI * 2.0).sin() * 0.5 + 0.5;
                (x2 + pulse * (x3 - x2), y_sus)
            }
            4 => {
                let x = x3 + frac * (x4 - x3);
                let y = y_sus + frac * (y_bot - y_sus);
                (x, y)
            }
            _ => continue,
        };

        // Stage color coded, glow ring in phosphor
        let dot_color = match stage {
            1 => PHOSPHOR,
            2 => Color32::from_rgb(240, 200, 40),
            3 => PHOSPHOR_MID,
            4 => Color32::from_rgb(220, 80, 60),
            _ => Color32::WHITE,
        };
        let p = Pos2::new(cx, cy);
        painter.circle_filled(p, 5.0, dot_color);
        painter.circle_stroke(
            p,
            7.0,
            Stroke::new(1.0, Color32::from_rgba_premultiplied(33, 255, 140, 120)),
        );
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
