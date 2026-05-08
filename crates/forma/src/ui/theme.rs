use egui::Color32;
use serde::{Deserialize, Serialize};

/// A complete visual theme for The Synth.
///
/// Organised in three layers:
///   1. Semantic backgrounds / borders / text  — used by SynthFrame and apply_to_egui
///   2. Domain-specific colors                 — scope glow, per-FX chips, sequencer cells
///   3. Geometry tokens                        — spacing scale, rounding, stroke widths
#[derive(Clone, Serialize, Deserialize)]
pub struct SynthTheme {
    pub name: String,

    // ── Semantic backgrounds (darkest → lightest) ──────────────────────────
    /// App window fill — the darkest background layer.
    pub bg_app: [u8; 3],
    /// Panel / card surface — one step above the app background.
    pub bg_surface: [u8; 3],
    /// Inset / input background — sits inside a surface (equal or darker).
    pub bg_sunken: [u8; 3],
    /// Global bar and transport strips — slightly distinct from the main app bg.
    pub bg_bar: [u8; 3],

    // ── Borders ────────────────────────────────────────────────────────────
    pub border: [u8; 3],
    pub border_focus: [u8; 3],

    // ── Text (semantic) ────────────────────────────────────────────────────
    pub text_primary: [u8; 3],
    pub text_secondary: [u8; 3],
    pub text_disabled: [u8; 3],

    // ── Accent ─────────────────────────────────────────────────────────────
    pub accent: [u8; 3],
    pub accent_dim: [u8; 3],

    // ── Special accents ─────────────────────────────────────────────────────
    pub accent_hard_sync: [u8; 3],
    pub accent_fm: [u8; 3],
    pub accent_ring: [u8; 3],
    pub accent_hold: [u8; 3],
    pub accent_walker: [u8; 3],
    pub accent_limiter: [u8; 3],

    // ── FX per-effect ───────────────────────────────────────────────────────
    pub fx_overdrive: [u8; 3],
    pub fx_distortion: [u8; 3],
    pub fx_chorus: [u8; 3],
    pub fx_delay: [u8; 3],
    pub fx_reverb: [u8; 3],
    pub fx_shimmer: [u8; 3],
    pub fx_crystallizer: [u8; 3],

    // ── Sequencer ───────────────────────────────────────────────────────────
    pub seq_step_on: [u8; 3],
    pub seq_step_off: [u8; 3],
    pub seq_current: [u8; 3],
    pub seq_note_bar_on: [u8; 3],
    pub seq_note_bar_off: [u8; 3],
    pub seq_chord_major: [u8; 3],
    pub seq_chord_minor: [u8; 3],
    pub seq_chord_dim: [u8; 3],
    pub seq_kb_major: [u8; 3],
    pub seq_kb_minor: [u8; 3],
    pub seq_kb_dim: [u8; 3],

    // ── Keyboard ────────────────────────────────────────────────────────────
    pub key_white_pressed: [u8; 3],
    pub key_black_pressed: [u8; 3],

    // ── Scope / visualizer ──────────────────────────────────────────────────
    pub scope_bg: [u8; 3],
    pub scope_zero: [u8; 3],
    pub scope_glow_outer: [u8; 4],
    pub scope_glow_mid: [u8; 4],
    pub scope_glow_core: [u8; 4],
    pub scope_label: [u8; 3],

    // ── Peak meter ──────────────────────────────────────────────────────────
    pub meter_bg: [u8; 3],
    pub meter_green: [u8; 3],
    pub meter_clip: [u8; 3],

    // ── ADSR visualizer ─────────────────────────────────────────────────────
    pub adsr_fill: [u8; 4],
    pub adsr_outline: [u8; 3],
    pub adsr_label: [u8; 4],
    pub adsr_cursor: [u8; 3],

    // ── Latency indicator ───────────────────────────────────────────────────
    pub latency_ok: [u8; 3],
    pub latency_warn: [u8; 3],
    pub latency_bad: [u8; 3],

    // ── Patch browser ───────────────────────────────────────────────────────
    pub patch_browser_model: [u8; 3],
    pub patch_load_fx_on: [u8; 3],

    // ── MIDI ────────────────────────────────────────────────────────────────
    pub midi_connected: [u8; 3],

    // ── Legacy panel backgrounds (kept for custom Painter calls) ────────────
    pub bg_panel: [u8; 3],
    pub bg_seq_bar: [u8; 3],
    pub bg_adsr: [u8; 3],

    // ── Geometry tokens (spacing, rounding, stroke) ─────────────────────────
    /// 4 px — tightest gap; used between related controls.
    pub sp_xs: f32,
    /// 8 px — standard item spacing.
    pub sp_sm: f32,
    /// 12 px — section inner margin.
    pub sp_md: f32,
    /// 16 px — comfortable breathing room.
    pub sp_lg: f32,
    /// 24 px — section-to-section gap.
    pub sp_xl: f32,
    /// Small corner radius — step buttons, chips.
    pub rounding_sm: f32,
    /// Medium corner radius — section cards.
    pub rounding_md: f32,
    /// Large corner radius — windows, popovers.
    pub rounding_lg: f32,
    /// Default border stroke width.
    pub stroke_ui: f32,
    /// Focused / hovered border stroke width.
    pub stroke_focus: f32,
    /// Active / pressed border stroke width.
    pub stroke_active: f32,
}

impl SynthTheme {
    // ── Color helpers ────────────────────────────────────────────────────────

    pub fn c(&self, rgb: &[u8; 3]) -> Color32 {
        Color32::from_rgb(rgb[0], rgb[1], rgb[2])
    }

    pub fn ca(&self, rgba: &[u8; 4]) -> Color32 {
        Color32::from_rgba_premultiplied(rgba[0], rgba[1], rgba[2], rgba[3])
    }

    /// Active/inactive — returns accent if `on`, else GRAY.
    pub fn active(&self, on: bool) -> Color32 {
        if on {
            self.c(&self.accent)
        } else {
            Color32::GRAY
        }
    }

    pub fn active_with(&self, on: bool, color: &[u8; 3]) -> Color32 {
        if on {
            self.c(color)
        } else {
            Color32::GRAY
        }
    }

    // ── egui integration ─────────────────────────────────────────────────────

    /// Apply this theme to egui's global `Visuals` and `Style`.
    ///
    /// Call once at the start of every `update()` frame so that all egui
    /// built-in widgets (buttons, sliders, labels, separators) automatically
    /// match the active theme without per-widget overrides.
    pub fn apply_to_egui(&self, ctx: &egui::Context) {
        use egui::style::WidgetVisuals;
        use egui::{Color32, CornerRadius, Margin, Shadow, Stroke, Vec2, Visuals};

        let bg_surface = self.c(&self.bg_surface);
        let bg_app = self.c(&self.bg_app);
        let bg_sunken = self.c(&self.bg_sunken);
        let border = self.c(&self.border);
        let border_focus = self.c(&self.border_focus);
        let text_primary = self.c(&self.text_primary);
        let text_secondary = self.c(&self.text_secondary);
        let accent = self.c(&self.accent);

        let round_md = CornerRadius::same(self.rounding_md as u8);

        // Slightly brighten a color for hover feedback.
        let lighten = |c: Color32, by: u8| {
            Color32::from_rgb(
                c.r().saturating_add(by),
                c.g().saturating_add(by),
                c.b().saturating_add(by),
            )
        };

        // Dim accent to use as active widget fill.
        let accent_fill =
            Color32::from_rgba_premultiplied(accent.r() / 5, accent.g() / 5, accent.b() / 5, 200);

        let wv = |bg: Color32, text: Color32, stroke_c: Color32, sw: f32| WidgetVisuals {
            bg_fill: bg,
            weak_bg_fill: bg,
            bg_stroke: Stroke::new(sw, stroke_c),
            corner_radius: round_md,
            fg_stroke: Stroke::new(1.0, text),
            expansion: 0.0,
        };

        let mut vis = Visuals::dark();

        // Background layers
        vis.panel_fill = bg_app;
        vis.window_fill = bg_surface;
        // Slider rails use extreme_bg_color — must be visibly distinct from bg_surface.
        // bg_sunken is often darker than bg_surface, making rails invisible; use a lightened surface instead.
        vis.extreme_bg_color = lighten(bg_surface, 22);
        vis.code_bg_color = bg_sunken;
        vis.faint_bg_color = bg_sunken;

        // Window chrome
        vis.window_corner_radius = CornerRadius::same(self.rounding_lg as u8);
        vis.window_stroke = Stroke::new(self.stroke_ui, border);
        vis.window_shadow = Shadow::NONE;
        vis.popup_shadow = Shadow::NONE;
        vis.menu_corner_radius = round_md;

        // Selection
        vis.selection.bg_fill =
            Color32::from_rgba_premultiplied(accent.r() / 5, accent.g() / 5, accent.b() / 5, 90);
        vis.selection.stroke = Stroke::new(self.stroke_focus, accent);
        vis.hyperlink_color = accent;

        // Widget states
        // inactive.bg_fill is used by Slider as the rail color — must be distinct from bg_surface.
        vis.widgets.noninteractive = wv(bg_surface, text_secondary, border, self.stroke_ui);
        vis.widgets.inactive = wv(
            lighten(bg_surface, 28),
            text_primary,
            border,
            self.stroke_ui,
        );
        vis.widgets.hovered = wv(
            lighten(bg_surface, 40),
            text_primary,
            border_focus,
            self.stroke_focus,
        );
        vis.widgets.active = wv(accent_fill, accent, accent, self.stroke_focus);
        vis.widgets.open = wv(
            lighten(bg_surface, 22),
            text_primary,
            border_focus,
            self.stroke_ui,
        );

        ctx.set_visuals(vis);

        // Layout / spacing
        let mut style = (*ctx.global_style()).clone();
        style.spacing.item_spacing = Vec2::new(self.sp_sm, self.sp_xs);
        style.spacing.window_margin = Margin::same(self.sp_md as i8);
        style.spacing.button_padding = Vec2::new(self.sp_sm, 3.0);
        style.spacing.menu_margin = Margin::same(self.sp_sm as i8);
        style.spacing.indent = self.sp_lg;
        style.spacing.interact_size = Vec2::new(40.0, 20.0);
        ctx.set_global_style(style);
    }
}

// ── Geometry defaults shared by all themes ───────────────────────────────────

fn geometry() -> (f32, f32, f32, f32, f32, f32, f32, f32, f32, f32, f32) {
    (
        4.0,  // sp_xs
        8.0,  // sp_sm
        12.0, // sp_md
        16.0, // sp_lg
        24.0, // sp_xl
        4.0,  // rounding_sm
        8.0,  // rounding_md
        12.0, // rounding_lg
        1.0,  // stroke_ui
        1.5,  // stroke_focus
        2.0,  // stroke_active
    )
}

// ── Built-in themes ──────────────────────────────────────────────────────────

/// Midnight — dark navy-blue with teal accent.
pub fn midnight() -> SynthTheme {
    let (sp_xs, sp_sm, sp_md, sp_lg, sp_xl, r_sm, r_md, r_lg, s_ui, s_foc, s_act) = geometry();
    SynthTheme {
        name: "Midnight".into(),

        bg_app: [6, 8, 12],
        bg_surface: [14, 18, 26],
        bg_sunken: [8, 11, 17],
        bg_bar: [10, 13, 19],

        border: [28, 35, 50],
        border_focus: [0, 160, 120],

        text_primary: [210, 218, 230],
        text_secondary: [110, 125, 145],
        text_disabled: [50, 60, 78],

        accent: [0, 220, 160],
        accent_dim: [0, 180, 130],

        accent_hard_sync: [255, 180, 0],
        accent_fm: [120, 180, 255],
        accent_ring: [255, 130, 200],
        accent_hold: [255, 200, 0],
        accent_walker: [100, 180, 255],
        accent_limiter: [0, 255, 0],

        fx_overdrive: [255, 140, 60],
        fx_distortion: [220, 60, 60],
        fx_chorus: [80, 200, 140],
        fx_delay: [80, 160, 255],
        fx_reverb: [170, 90, 240],
        fx_shimmer: [120, 200, 255],
        fx_crystallizer: [255, 170, 90],

        seq_step_on: [0, 180, 120],
        seq_step_off: [40, 40, 55],
        seq_current: [255, 200, 50],
        seq_note_bar_on: [0, 120, 80],
        seq_note_bar_off: [40, 50, 55],
        seq_chord_major: [0, 100, 70],
        seq_chord_minor: [60, 80, 140],
        seq_chord_dim: [120, 50, 50],
        seq_kb_major: [30, 80, 55],
        seq_kb_minor: [40, 55, 100],
        seq_kb_dim: [80, 35, 35],

        key_white_pressed: [100, 180, 255],
        key_black_pressed: [60, 120, 200],

        scope_bg: [4, 10, 7],
        scope_zero: [12, 28, 18],
        scope_glow_outer: [0, 160, 90, 14],
        scope_glow_mid: [0, 210, 130, 45],
        scope_glow_core: [55, 255, 165, 230],
        scope_label: [60, 100, 80],

        meter_bg: [10, 15, 20],
        meter_green: [0, 200, 80],
        meter_clip: [255, 50, 30],

        adsr_fill: [0, 160, 100, 30],
        adsr_outline: [0, 200, 130],
        adsr_label: [80, 160, 110, 180],
        adsr_cursor: [0, 255, 160],

        latency_ok: [0, 180, 120],
        latency_warn: [200, 180, 0],
        latency_bad: [200, 70, 50],

        patch_browser_model: [100, 180, 255],
        patch_load_fx_on: [255, 180, 60],

        midi_connected: [0, 220, 120],

        bg_panel: [10, 15, 20],
        bg_seq_bar: [25, 25, 35],
        bg_adsr: [8, 14, 10],

        sp_xs,
        sp_sm,
        sp_md,
        sp_lg,
        sp_xl,
        rounding_sm: r_sm,
        rounding_md: r_md,
        rounding_lg: r_lg,
        stroke_ui: s_ui,
        stroke_focus: s_foc,
        stroke_active: s_act,
    }
}

/// Winamp Classic — dark grey with vivid green.
pub fn winamp_classic() -> SynthTheme {
    let (sp_xs, sp_sm, sp_md, sp_lg, sp_xl, r_sm, r_md, r_lg, s_ui, s_foc, s_act) = geometry();
    SynthTheme {
        name: "Winamp Classic".into(),

        bg_app: [10, 10, 10],
        bg_surface: [22, 22, 22],
        bg_sunken: [14, 14, 14],
        bg_bar: [18, 18, 18],

        border: [42, 42, 42],
        border_focus: [0, 200, 0],

        text_primary: [215, 215, 215],
        text_secondary: [130, 130, 130],
        text_disabled: [65, 65, 65],

        accent: [0, 230, 0],
        accent_dim: [0, 180, 0],

        accent_hard_sync: [255, 200, 0],
        accent_fm: [150, 200, 60],
        accent_ring: [255, 150, 60],
        accent_hold: [255, 220, 0],
        accent_walker: [150, 200, 60],
        accent_limiter: [0, 230, 0],

        fx_overdrive: [255, 170, 0],
        fx_distortion: [255, 80, 40],
        fx_chorus: [0, 200, 100],
        fx_delay: [80, 180, 255],
        fx_reverb: [200, 120, 255],
        fx_shimmer: [100, 220, 255],
        fx_crystallizer: [255, 200, 60],

        seq_step_on: [0, 200, 0],
        seq_step_off: [40, 40, 40],
        seq_current: [255, 220, 0],
        seq_note_bar_on: [0, 140, 0],
        seq_note_bar_off: [45, 45, 45],
        seq_chord_major: [0, 120, 0],
        seq_chord_minor: [60, 90, 120],
        seq_chord_dim: [140, 60, 40],
        seq_kb_major: [20, 70, 20],
        seq_kb_minor: [40, 50, 80],
        seq_kb_dim: [80, 40, 30],

        key_white_pressed: [0, 220, 0],
        key_black_pressed: [0, 160, 0],

        scope_bg: [6, 6, 6],
        scope_zero: [20, 30, 20],
        scope_glow_outer: [0, 160, 0, 14],
        scope_glow_mid: [0, 210, 0, 45],
        scope_glow_core: [55, 255, 55, 230],
        scope_label: [80, 120, 80],

        meter_bg: [14, 14, 14],
        meter_green: [0, 220, 0],
        meter_clip: [255, 40, 20],

        adsr_fill: [0, 160, 0, 30],
        adsr_outline: [0, 200, 0],
        adsr_label: [80, 160, 80, 180],
        adsr_cursor: [0, 255, 0],

        latency_ok: [0, 200, 0],
        latency_warn: [220, 200, 0],
        latency_bad: [220, 60, 40],

        patch_browser_model: [150, 200, 60],
        patch_load_fx_on: [255, 200, 0],

        midi_connected: [0, 230, 0],

        bg_panel: [18, 18, 18],
        bg_seq_bar: [30, 30, 30],
        bg_adsr: [12, 12, 12],

        sp_xs,
        sp_sm,
        sp_md,
        sp_lg,
        sp_xl,
        rounding_sm: r_sm,
        rounding_md: r_md,
        rounding_lg: r_lg,
        stroke_ui: s_ui,
        stroke_focus: s_foc,
        stroke_active: s_act,
    }
}

/// Phosphor — CRT green-on-black.
pub fn phosphor() -> SynthTheme {
    let (sp_xs, sp_sm, sp_md, sp_lg, sp_xl, r_sm, r_md, r_lg, s_ui, s_foc, s_act) = geometry();
    SynthTheme {
        name: "Phosphor".into(),

        bg_app: [1, 5, 2],
        bg_surface: [3, 11, 5],
        bg_sunken: [2, 7, 3],
        bg_bar: [2, 8, 3],

        border: [12, 40, 18],
        border_focus: [25, 220, 100],

        text_primary: [170, 235, 190],
        text_secondary: [70, 140, 88],
        text_disabled: [28, 65, 38],

        accent: [30, 255, 120],
        accent_dim: [20, 200, 90],

        accent_hard_sync: [200, 255, 80],
        accent_fm: [80, 255, 180],
        accent_ring: [160, 255, 100],
        accent_hold: [220, 255, 60],
        accent_walker: [80, 255, 180],
        accent_limiter: [30, 255, 120],

        fx_overdrive: [200, 255, 60],
        fx_distortion: [255, 160, 60],
        fx_chorus: [40, 255, 160],
        fx_delay: [60, 200, 255],
        fx_reverb: [140, 180, 255],
        fx_shimmer: [80, 240, 255],
        fx_crystallizer: [200, 255, 100],

        seq_step_on: [20, 220, 100],
        seq_step_off: [10, 30, 18],
        seq_current: [200, 255, 80],
        seq_note_bar_on: [10, 160, 70],
        seq_note_bar_off: [12, 35, 20],
        seq_chord_major: [10, 140, 60],
        seq_chord_minor: [30, 80, 100],
        seq_chord_dim: [100, 60, 30],
        seq_kb_major: [10, 60, 30],
        seq_kb_minor: [20, 40, 60],
        seq_kb_dim: [50, 30, 20],

        key_white_pressed: [40, 255, 140],
        key_black_pressed: [20, 180, 90],

        scope_bg: [2, 6, 3],
        scope_zero: [8, 24, 12],
        scope_glow_outer: [0, 180, 80, 16],
        scope_glow_mid: [10, 230, 110, 50],
        scope_glow_core: [60, 255, 150, 240],
        scope_label: [40, 100, 60],

        meter_bg: [4, 10, 6],
        meter_green: [20, 230, 100],
        meter_clip: [255, 80, 30],

        adsr_fill: [0, 180, 80, 25],
        adsr_outline: [20, 220, 100],
        adsr_label: [60, 180, 100, 180],
        adsr_cursor: [40, 255, 140],

        latency_ok: [20, 220, 100],
        latency_warn: [200, 220, 40],
        latency_bad: [220, 80, 40],

        patch_browser_model: [80, 255, 180],
        patch_load_fx_on: [220, 255, 60],

        midi_connected: [30, 255, 120],

        bg_panel: [2, 8, 4],
        bg_seq_bar: [8, 20, 12],
        bg_adsr: [4, 12, 6],

        sp_xs,
        sp_sm,
        sp_md,
        sp_lg,
        sp_xl,
        rounding_sm: r_sm,
        rounding_md: r_md,
        rounding_lg: r_lg,
        stroke_ui: s_ui,
        stroke_focus: s_foc,
        stroke_active: s_act,
    }
}

pub fn builtin_themes() -> Vec<SynthTheme> {
    vec![midnight(), winamp_classic(), phosphor()]
}
