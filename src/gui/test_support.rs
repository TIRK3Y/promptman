//! Helpers for GUI tests: an isolated app state and headless rendering.
use std::sync::mpsc::Receiver;
use std::time::Duration;

use eframe::egui::{self, Pos2, Rect, Vec2};

use crate::app::{Event, Shared};
use crate::core_engine::engine::Engine;

/// Window sizes from a narrow phone-like width up to full HD.
pub const SIZES: [(f32, f32); 4] = [(360.0, 640.0), (800.0, 600.0), (1280.0, 800.0), (1920.0, 1080.0)];

/// Nothing listens here, so requests fail fast with "connection refused" and never reach a real provider.
pub const NOWHERE: &str = "http://127.0.0.1:9/v1";

/// App state on an in-memory database; its "custom" provider points at [`NOWHERE`].
pub fn shared() -> (Shared, Receiver<Event>) {
    Shared::for_test(Engine::for_test(NOWHERE))
}

/// Draws `f` for a few frames at every size in [`SIZES`], in dark and light themes.
/// A layout panic (such as a negative size) fails the calling test.
pub fn render_all(mut f: impl FnMut(&mut egui::Ui)) {
    let ctx = egui::Context::default();
    super::theme::apply(&ctx);
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        ctx.set_theme(theme);
        for (w, h) in SIZES {
            render_frames(&ctx, Vec2::new(w, h), &mut f);
        }
    }
}

/// Runs three frames: panels size themselves from the previous frame, so problems can show up late.
pub fn render_frames(ctx: &egui::Context, size: Vec2, f: &mut impl FnMut(&mut egui::Ui)) {
    for _ in 0..3 {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), ..Default::default() };
        // No GPU here: drop the texture uploads a real window would apply.
        ctx.run_ui(input, &mut *f).textures_delta.clear();
    }
}

/// The next event from a background task.
pub fn next_event(rx: &Receiver<Event>) -> Event {
    rx.recv_timeout(Duration::from_secs(15)).expect("no event within 15 s")
}
