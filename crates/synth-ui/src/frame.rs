use crate::theme::SynthTheme;
use egui::{Frame, Margin, Stroke};

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
        let h = theme.sp_md as i8;
        Frame::none()
            .fill(theme.c(&theme.bg_bar))
            .inner_margin(Margin {
                left: h,
                right: h,
                top: 6,
                bottom: 6,
            })
    }

    /// Transport / keyboard strip variant — tighter vertical margin.
    pub fn transport(theme: &SynthTheme) -> Frame {
        let x = theme.sp_sm as i8;
        let y = theme.sp_xs as i8;
        Frame::none()
            .fill(theme.c(&theme.bg_bar))
            .inner_margin(Margin {
                left: x,
                right: x,
                top: y,
                bottom: y,
            })
    }

    /// Section card — the primary container for editing zones.
    ///
    /// Provides a raised surface with a subtle border and consistent padding.
    /// Use this to wrap OSC panels, filter section, FX chain, etc.
    pub fn section(theme: &SynthTheme) -> Frame {
        Frame::none()
            .fill(theme.c(&theme.bg_surface))
            .rounding(theme.rounding_md)
            .stroke(Stroke::new(theme.stroke_ui, theme.c(&theme.border)))
            .inner_margin(theme.sp_sm)
    }

    /// Inset — a darker sub-region inside a section.
    ///
    /// Use for control groups, value readouts, or any area that should sit
    /// visually "below" the surrounding surface.
    pub fn inset(theme: &SynthTheme) -> Frame {
        Frame::none()
            .fill(theme.c(&theme.bg_sunken))
            .rounding(theme.rounding_sm)
            .inner_margin(theme.sp_xs)
    }

    /// Screen — dark background for visualizers (scope, spectrum, etc.).
    pub fn screen(theme: &SynthTheme) -> Frame {
        Frame::none()
            .fill(theme.c(&theme.scope_bg))
            .rounding(theme.rounding_sm)
            .stroke(Stroke::new(theme.stroke_ui, theme.c(&theme.border)))
            .inner_margin(theme.sp_xs)
    }

    /// App background — transparent fill used on CentralPanel and side panels
    /// so that the app-level `bg_app` shows through without adding a border.
    pub fn app_bg(theme: &SynthTheme) -> Frame {
        Frame::none().fill(theme.c(&theme.bg_app))
    }
}
