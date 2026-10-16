//! Shared palette, global style, and small layout helpers for the GUI.
use eframe::egui::{self, Color32, CornerRadius, FontId, Frame, Margin, RichText, Stroke, Style, TextStyle, Theme, Ui};

pub struct Palette {
    pub bg: Color32,
    pub surface: Color32,
    pub input: Color32,
    pub control: Color32,
    pub control_hover: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
}

pub const DARK: Palette = Palette {
    bg: Color32::from_rgb(13, 15, 20),
    surface: Color32::from_rgb(22, 25, 32),
    input: Color32::from_rgb(16, 18, 24),
    control: Color32::from_rgb(32, 36, 45),
    control_hover: Color32::from_rgb(42, 47, 58),
    border: Color32::from_rgb(40, 44, 54),
    text: Color32::from_rgb(230, 232, 236),
    muted: Color32::from_rgb(139, 147, 161),
    accent: Color32::from_rgb(88, 140, 255),
    success: Color32::from_rgb(63, 185, 80),
    warning: Color32::from_rgb(210, 153, 34),
    danger: Color32::from_rgb(248, 81, 73),
};

pub const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(246, 247, 249),
    surface: Color32::WHITE,
    input: Color32::from_rgb(250, 251, 252),
    control: Color32::from_rgb(240, 242, 245),
    control_hover: Color32::from_rgb(229, 232, 237),
    border: Color32::from_rgb(222, 226, 231),
    text: Color32::from_rgb(31, 35, 40),
    muted: Color32::from_rgb(101, 109, 118),
    accent: Color32::from_rgb(37, 99, 235),
    success: Color32::from_rgb(21, 128, 61),
    warning: Color32::from_rgb(180, 83, 9),
    danger: Color32::from_rgb(220, 38, 38),
};

pub fn palette(ui: &Ui) -> &'static Palette {
    if ui.visuals().dark_mode {
        &DARK
    } else {
        &LIGHT
    }
}

/// Installs the dark and light styles; egui picks one from the theme preference.
pub fn apply(ctx: &egui::Context) {
    ctx.style_mut_of(Theme::Dark, |s| style(s, &DARK));
    ctx.style_mut_of(Theme::Light, |s| style(s, &LIGHT));
}

fn style(s: &mut Style, p: &Palette) {
    s.text_styles = [
        (TextStyle::Heading, FontId::proportional(20.0)),
        (TextStyle::Body, FontId::proportional(14.0)),
        (TextStyle::Button, FontId::proportional(14.0)),
        (TextStyle::Small, FontId::proportional(12.0)),
        (TextStyle::Monospace, FontId::monospace(13.0)),
    ]
    .into();
    s.spacing.item_spacing = egui::vec2(8.0, 8.0);
    s.spacing.button_padding = egui::vec2(12.0, 6.0);
    s.spacing.interact_size.y = 28.0;

    let v = &mut s.visuals;
    v.panel_fill = p.bg;
    v.window_fill = p.surface;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.faint_bg_color = p.control; // striped grid rows
    v.extreme_bg_color = p.input; // text edits, progress track
    v.code_bg_color = p.control;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.danger;
    v.selection.bg_fill = p.accent.gamma_multiply(0.3);
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(8);

    let w = &mut v.widgets;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.bg_fill = p.control;
    w.inactive.weak_bg_fill = p.control;
    w.inactive.bg_stroke = Stroke::new(1.0, p.border);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.hovered.bg_fill = p.control_hover;
    w.hovered.weak_bg_fill = p.control_hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.muted);
    w.hovered.fg_stroke = Stroke::new(1.0, p.text);
    w.active.weak_bg_fill = p.control_hover;
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.open.weak_bg_fill = p.control_hover;
    for ws in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        ws.corner_radius = CornerRadius::same(6);
        ws.expansion = 0.0;
    }
}

/// Bordered surface that stretches to the available width.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = palette(ui);
    Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Panel or page title with a one-line description.
pub fn title(ui: &mut Ui, title: &str, subtitle: &str) {
    let muted = palette(ui).muted;
    ui.label(RichText::new(title).heading().strong());
    ui.label(RichText::new(subtitle).color(muted));
}

/// Small uppercase label above a group of controls.
pub fn caption(ui: &mut Ui, text: &str) {
    let muted = palette(ui).muted;
    ui.label(RichText::new(text.to_uppercase()).size(11.0).strong().color(muted));
}

/// Rounded, tinted badge.
pub fn pill(ui: &mut Ui, text: &str, color: Color32) {
    Frame::new()
        .fill(color.gamma_multiply(0.15))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| ui.label(RichText::new(text).small().strong().color(color)));
}

/// Caption with a large value underneath.
pub fn stat(ui: &mut Ui, label: &str, value: String) {
    ui.vertical(|ui| {
        caption(ui, label);
        ui.label(RichText::new(value).size(22.0).strong());
    });
}

/// Small filled status dot.
pub fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

/// Green under the warning threshold, amber above it, red at or over the cap.
pub fn budget_color(p: &Palette, ratio: f64, warn_percent: f32) -> Color32 {
    if ratio >= 1.0 {
        p.danger
    } else if ratio * 100.0 >= warn_percent as f64 {
        p.warning
    } else {
        p.success
    }
}

/// Full-width tinted strip for notices.
pub fn banner<R>(ui: &mut Ui, color: Color32, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(color.gamma_multiply(0.12))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.5)))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_wrapped(add).inner
        })
        .inner
}

/// Opens a file or folder with the system's default app.
pub fn open_path(path: &std::path::Path) {
    #[cfg(target_os = "windows")]
    let cmd = "explorer";
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let cmd = "xdg-open";
    let _ = std::process::Command::new(cmd).arg(path).spawn();
}

/// Lays out `add` in exactly the space available and reports exactly that size.
/// Resizable panels size themselves from their contents, so a child that is
/// even a pixel too wide makes the panel grow every frame; this stops that.
pub fn contained<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let rect = ui.available_rect_before_wrap();
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(*ui.layout()));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    let r = add(&mut child);
    ui.allocate_rect(rect, egui::Sense::hover());
    r
}

/// Square brand mark for the top bar.
pub fn logo(ui: &mut Ui) {
    let p = palette(ui);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(7), p.accent);
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "P", FontId::proportional(15.0), Color32::WHITE);
}

/// Top-bar tab: muted text, hover highlight, accent underline when selected.
pub fn nav_tab(ui: &mut Ui, selected: bool, label: &str) -> egui::Response {
    let p = palette(ui);
    let text = RichText::new(label).size(14.0);
    let text = if selected { text.strong() } else { text };
    let galley = egui::WidgetText::from(text).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        TextStyle::Button,
    );
    let size = egui::vec2(galley.size().x + 24.0, ui.available_height());
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let painter = ui.painter();
    if resp.hovered() && !selected {
        painter.rect_filled(rect.shrink2(egui::vec2(2.0, 10.0)), CornerRadius::same(6), p.control);
    }
    let color = if selected || resp.hovered() { p.text } else { p.muted };
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
    if selected {
        let bar = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 10.0, rect.bottom() - 2.5),
            egui::pos2(rect.right() - 10.0, rect.bottom()),
        );
        painter.rect_filled(bar, CornerRadius::same(1), p.accent);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Small rounded status chip. Its row follows the parent's direction, so in a
/// right-to-left area add items last-to-first.
pub fn chip(ui: &mut Ui, add: impl FnOnce(&mut Ui)) -> egui::Response {
    let p = palette(ui);
    Frame::new()
        .fill(p.control)
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::symmetric(10, 4))
        .show(ui, |ui| ui.horizontal(add))
        .response
}
