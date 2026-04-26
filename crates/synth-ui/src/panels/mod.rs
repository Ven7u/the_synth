pub mod fx_chain;
pub mod modulation;
pub mod oscillators;

pub use fx_chain::ui_fx_chain;
pub use modulation::{draw_adsr_visualizer, ui_adsr_panel, ui_filter_panel, ui_lfo2_panel, ui_lfo_panel};
pub use oscillators::{update_freq_mult, update_unison, ui_mixer_panel, ui_osc_panel};
