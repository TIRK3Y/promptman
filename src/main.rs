// Prevents console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use promptman::app::PromptManApp;
use promptman::core_engine::engine::Engine;
use promptman::gui::theme;

fn main() -> Result<(), eframe::Error> {
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    // Background runtime for network requests; tasks are spawned from the UI thread.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to initialize multi-threaded Tokio runtime");
    let _enter = rt.enter();

    let (engine, notes) = match Engine::open() {
        Ok(e) => e,
        Err(e) => {
            // No console in release builds, so show the error in a dialog.
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("PromptMan could not start")
                .set_description(e)
                .show();
            std::process::exit(1);
        }
    };

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1360.0, 860.0])
        .with_min_inner_size([1024.0, 640.0])
        .with_title("PromptMan");
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icons/app-icon.png")) {
        viewport = viewport.with_icon(icon);
    }

    eframe::run_native(
        "PromptMan",
        eframe::NativeOptions { viewport, ..Default::default() },
        Box::new(|cc| {
            // Theme follows the OS until toggled in the top bar (persisted).
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(PromptManApp::new(cc, engine, notes)))
        }),
    )
}
