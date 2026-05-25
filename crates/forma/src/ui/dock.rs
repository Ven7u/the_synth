use crate::SynthApp;
use egui::WidgetText;
use egui_dock::{DockState, NodeIndex, Split, TabViewer};

/// Each dockable panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    Oscillators,
    Modulation,
    Filter,
    Sequencer,
    ArpWalker,
    FxChain,
    Scope,
    Midi,
}

impl Tab {
    pub fn title(self) -> &'static str {
        match self {
            Tab::Oscillators => "Oscillators",
            Tab::Modulation => "Modulation",
            Tab::Filter => "Filter & Envelopes",
            Tab::Sequencer => "Sequencer",
            Tab::ArpWalker => "Arp & Walker",
            Tab::FxChain => "FX Chain",
            Tab::Scope => "Oscilloscope",
            Tab::Midi => "MIDI & Latency",
        }
    }

    pub const ALL: &[Tab] = &[
        Tab::Oscillators,
        Tab::Modulation,
        Tab::Filter,
        Tab::Sequencer,
        Tab::ArpWalker,
        Tab::FxChain,
        Tab::Scope,
        Tab::Midi,
    ];
}

/// Build the default dock layout.
///
/// ```text
/// ┌─────────────────────────┬───────────────────────┐
/// │  Oscillators            │  Oscilloscope         │
/// ├────────────┬────────────┼───────────────────────┤
/// │ Modulation │Arp & Walker│  FX Chain             │
/// │ & Filter   │   (tabbed) │                       │
/// ├────────────┴────────────┴───────────────────────┤
/// │  Keyboard  │  Sequencer    (tabbed)              │
/// └─────────────────────────────────────────────────┘
/// ```
pub fn default_dock_state() -> DockState<Tab> {
    // Start with Oscillators as root.
    let mut state = DockState::new(vec![Tab::Oscillators]);
    let surface = state.main_surface_mut();

    // 1. Split bottom from root: Sequencer + ArpWalker tabbed — bottom 32%.
    let [top, _bottom] = surface.split_below(
        NodeIndex::root(),
        0.68,
        vec![Tab::Sequencer, Tab::ArpWalker],
    );

    // 2. In top area, split right: Oscilloscope — right takes 40%.
    let [top_left, top_right] = surface.split_right(top, 0.60, vec![Tab::Scope]);

    // 3. Split top-left vertically: Modulation + Filter tabbed below Oscillators.
    let [_osc, _mod] = surface.split_below(top_left, 0.55, vec![Tab::Modulation, Tab::Filter]);

    // 4. Split top-right vertically: FX Chain below Oscilloscope.
    let [_scope, _fx] = surface.split_below(top_right, 0.50, vec![Tab::FxChain]);

    state
}

/// Render the full egui-dock area — used by Studio mode and the LIVE per-track view.
impl crate::SynthApp {
    pub fn ui_synth_dock(&mut self, ui: &mut egui::Ui) {
        if self.reset_layout_pending {
            self.dock_state = default_dock_state();
            self.reset_layout_pending = false;
        }
        let mut dock_state =
            std::mem::replace(&mut self.dock_state, egui_dock::DockState::new(vec![]));
        let mut s = egui_dock::Style::from_egui(ui.style());
        s.separator.width = 6.0;
        s.separator.color_idle = egui::Color32::TRANSPARENT;
        s.separator.color_hovered = egui::Color32::from_black_alpha(60);
        s.separator.color_dragged = egui::Color32::from_black_alpha(100);
        s.dock_area_padding = Some(egui::Margin::same(6i8));
        let rm = self.theme.rounding_md as u8;
        let r_top = egui::CornerRadius {
            nw: rm,
            ne: rm,
            sw: 0,
            se: 0,
        };
        let r_body = egui::CornerRadius {
            nw: 0,
            ne: rm,
            sw: rm,
            se: rm,
        };
        let bg_surface = self.theme.c(&self.theme.bg_surface);
        let border = self.theme.c(&self.theme.border);
        let text_pri = self.theme.c(&self.theme.text_primary);
        let text_sec = self.theme.c(&self.theme.text_secondary);
        let accent = self.theme.c(&self.theme.accent);
        s.tab.tab_body.corner_radius = r_body;
        s.tab.tab_body.stroke = egui::Stroke::new(self.theme.stroke_ui, border);
        s.tab_bar.bg_fill = egui::Color32::TRANSPARENT;
        s.tab_bar.hline_color = egui::Color32::TRANSPARENT;
        s.tab_bar.corner_radius = r_body;
        s.tab_bar.height = 28.0;
        s.tab.active = egui_dock::TabInteractionStyle {
            corner_radius: r_top,
            bg_fill: bg_surface,
            text_color: text_pri,
            outline_color: accent,
        };
        s.tab.focused = egui_dock::TabInteractionStyle {
            corner_radius: r_top,
            bg_fill: bg_surface,
            text_color: accent,
            outline_color: accent,
        };
        s.tab.inactive = egui_dock::TabInteractionStyle {
            corner_radius: r_top,
            bg_fill: egui::Color32::TRANSPARENT,
            text_color: text_sec,
            outline_color: egui::Color32::TRANSPARENT,
        };
        s.tab.hovered = egui_dock::TabInteractionStyle {
            corner_radius: r_top,
            bg_fill: bg_surface,
            text_color: text_pri,
            outline_color: border,
        };
        egui_dock::DockArea::new(&mut dock_state)
            .style(s)
            .show_inside(ui, &mut SynthTabViewer { app: self });
        self.dock_state = dock_state;
    }
}

/// Tab viewer that delegates rendering to SynthApp methods.
pub struct SynthTabViewer<'a> {
    pub app: &'a mut SynthApp,
}

impl<'a> TabViewer for SynthTabViewer<'a> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Tab) -> WidgetText {
        tab.title().into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        match tab {
            Tab::Oscillators => {
                ui.columns(4, |cols| {
                    self.app.ui_osc_panel(&mut cols[0], 0);
                    self.app.ui_osc_panel(&mut cols[1], 1);
                    self.app.ui_osc_panel(&mut cols[2], 2);
                    self.app.ui_mixer_panel(&mut cols[3]);
                });
            }
            Tab::Modulation => {
                ui.vertical(|ui| {
                    ui.columns(2, |cols| {
                        self.app.ui_lfo_panel(&mut cols[0]);
                        self.app.ui_lfo2_panel(&mut cols[1]);
                    });
                    self.app.ui_pulse_panel(ui);
                });
            }
            Tab::Filter => {
                ui.columns(3, |cols| {
                    self.app.ui_filter_panel(&mut cols[0]);
                    self.app.ui_adsr_panel(
                        &mut cols[1],
                        "Filter Env",
                        &mut [0usize, 1, 2, 3],
                        true,
                    );
                    self.app
                        .ui_adsr_panel(&mut cols[2], "Amp Env", &mut [0usize, 1, 2, 3], false);
                });
            }
            Tab::Sequencer => {
                self.app.ui_sequencer_panel(ui);
            }
            Tab::ArpWalker => {
                ui.columns(2, |cols| {
                    self.app.ui_arp_panel(&mut cols[0]);
                    self.app.ui_walker_panel(&mut cols[1]);
                });
            }
            Tab::FxChain => {
                self.app.ui_fx_chain(ui);
            }
            Tab::Scope => {
                self.app.ui_oscilloscope(ui);
            }
            Tab::Midi => {
                self.app.ui_midi_panel(ui);
                ui.separator();
                super::scope::draw_latency_bar(
                    ui,
                    &self.app.engine,
                    self.app.engine.amp_attack(),
                    &self.app.theme,
                );
            }
        }
    }
}
