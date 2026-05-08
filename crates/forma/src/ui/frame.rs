use super::theme::SynthTheme;
use egui::{CornerRadius, Frame, Margin, Stroke};

/// Typed `egui::Frame` factories that read directly from `SynthTheme` tokens.
///
/// Every zone and surface in the UI should use one of these variants instead of
/// building frames ad-hoc. Changing a token in the theme automatically propagates
/// to every component that uses the corresponding variant.
///
/// Usage:
/// ```ignore
/// SynthFrame::section(&theme).show(ui, |ui| { /* section content */ });
/// ```
pub struct SynthFrame;

impl SynthFrame {
    /// Global bar and transport strips — full-bleed, no border, no rounding.
    pub fn bar(theme: &SynthTheme) -> Frame {
        Frame::new()
            .fill(theme.c(&theme.bg_bar))
            .inner_margin(Margin::symmetric(theme.sp_md as i8, 6))
    }

    /// Transport / keyboard strip variant — tighter vertical margin.
    pub fn transport(theme: &SynthTheme) -> Frame {
        Frame::new()
            .fill(theme.c(&theme.bg_bar))
            .inner_margin(Margin::symmetric(theme.sp_sm as i8, theme.sp_xs as i8))
    }

    /// Section card — the primary container for editing zones.
    ///
    /// Provides a raised surface with a subtle border and consistent padding.
    /// Use this to wrap OSC panels, filter section, FX chain, etc.
    pub fn section(theme: &SynthTheme) -> Frame {
        Frame::new()
            .fill(theme.c(&theme.bg_surface))
            .corner_radius(CornerRadius::same(theme.rounding_md as u8))
            .stroke(Stroke::new(theme.stroke_ui, theme.c(&theme.border)))
            .inner_margin(Margin::same(theme.sp_sm as i8))
            .outer_margin(Margin::same(theme.sp_xs as i8))
    }

    /// Inset — a darker sub-region inside a section.
    ///
    /// Use for control groups, value readouts, or any area that should sit
    /// visually "below" the surrounding surface.
    pub fn inset(theme: &SynthTheme) -> Frame {
        Frame::new()
            .fill(theme.c(&theme.bg_sunken))
            .corner_radius(CornerRadius::same(theme.rounding_sm as u8))
            .inner_margin(Margin::same(theme.sp_xs as i8))
    }

    /// Screen — dark background for visualizers (scope, spectrum, etc.).
    pub fn screen(theme: &SynthTheme) -> Frame {
        Frame::new()
            .fill(theme.c(&theme.scope_bg))
            .corner_radius(CornerRadius::same(theme.rounding_sm as u8))
            .stroke(Stroke::new(theme.stroke_ui, theme.c(&theme.border)))
            .inner_margin(Margin::same(theme.sp_xs as i8))
    }

    /// App background — transparent fill used on CentralPanel and side panels
    /// so that the app-level `bg_app` shows through without adding a border.
    pub fn app_bg(theme: &SynthTheme) -> Frame {
        Frame::new().fill(theme.c(&theme.bg_app))
    }
}
