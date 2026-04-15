use egui::WidgetText;
use egui_dock::{DockState, NodeIndex, Split, TabViewer};
use crate::SynthApp;

/// Each dockable panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    Oscillators,
    Modulation,
    Keyboard,
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
            Tab::Modulation  => "Modulation & Filter",
            Tab::Keyboard    => "Keyboard",
            Tab::Sequencer   => "Sequencer",
            Tab::ArpWalker   => "Arp & Walker",
            Tab::FxChain     => "FX Chain",
            Tab::Scope       => "Oscilloscope",
            Tab::Midi        => "MIDI & Latency",
        }
    }

    pub const ALL: &[Tab] = &[
        Tab::Oscillators,
        Tab::Modulation,
        Tab::Keyboard,
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

    // 1. Split bottom from root: Keyboard + Sequencer (tabbed) — bottom 35%.
    let [top, bottom] = surface.split_below(
        NodeIndex::root(), 0.65,
        vec![Tab::Keyboard, Tab::Sequencer],
    );

    // 2. In top area, split right: Oscilloscope — right takes 42%.
    let [top_left, top_right] = surface.split_right(top, 0.58, vec![Tab::Scope]);

    // 3. Split top-left vertically: Modulation + Arp (tabbed) below Oscillators.
    let [_osc, _mod] = surface.split_below(
        top_left, 0.50,
        vec![Tab::Modulation, Tab::ArpWalker],
    );

    // 4. Split top-right vertically: FX Chain below Oscilloscope.
    let [_scope, _fx] = surface.split_below(top_right, 0.50, vec![Tab::FxChain]);

    state
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
                ui.columns(4, |cols| {
                    self.app.ui_lfo_panel(&mut cols[0]);
                    self.app.ui_filter_panel(&mut cols[1]);
                    self.app.ui_adsr_panel(&mut cols[2], "Filter Env", &mut [0usize, 1, 2, 3], true);
                    self.app.ui_adsr_panel(&mut cols[3], "Amp Env", &mut [0usize, 1, 2, 3], false);
                });
            }
            Tab::Keyboard => {
                self.app.ui_keyboard_panel(ui);
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
                super::scope::draw_latency_bar(ui, &self.app.state, self.app.amp_adsr[0], &self.app.theme);
            }
        }
    }
}
