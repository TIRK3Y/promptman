//! Settings tab: keys, endpoints, Ollama models, budget, pricing, data.
use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui::{self, Frame, Margin, ProgressBar, RichText, Ui};

use super::chat::{self, ChatState};
use super::theme::{self, palette};
use crate::app::{Action, Event, Shared};
use crate::core_engine::engine::{Budget, Engine};
use crate::core_engine::local_bridge::LocalBridge;
use crate::core_engine::providers::PROVIDERS;
use crate::core_engine::secrets;

pub struct SettingsState {
    key_inputs: HashMap<&'static str, String>,
    url_inputs: HashMap<&'static str, String>,
    budget: Budget,
    pull_name: String,
    pull_progress: Option<(String, Option<f32>)>,
    ollama_busy: bool,
}

impl SettingsState {
    pub fn new(engine: &Engine) -> Self {
        Self {
            key_inputs: HashMap::new(),
            url_inputs: PROVIDERS.iter().map(|p| (p.id, engine.base_url(p.id))).collect(),
            budget: engine.budget(),
            pull_name: String::new(),
            pull_progress: None,
            ollama_busy: false,
        }
    }

    pub fn handle(&mut self, event: Event, shared: &mut Shared) {
        match event {
            Event::PullProgress { status, pct } => self.pull_progress = Some((status, pct)),
            Event::OllamaOpDone(result) => {
                self.ollama_busy = false;
                self.pull_progress = None;
                match result {
                    Ok(msg) => shared.notice(msg, false),
                    Err(e) => shared.notice(e, true),
                }
                shared.actions.push(Action::RefreshOllama);
            }
            _ => {}
        }
    }

    fn ollama_op(&mut self, shared: &Shared, op: OllamaOp, model: String) {
        self.ollama_busy = true;
        let notify = shared.notify.clone();
        let host = shared.engine.base_url("ollama");
        tokio::spawn(async move {
            let bridge = LocalBridge::new(&host);
            let result = match op {
                OllamaOp::Pull => {
                    let progress = notify.clone();
                    bridge
                        .pull(&model, |status, pct| {
                            progress.send(Event::PullProgress { status: status.to_string(), pct })
                        })
                        .await
                        .map(|_| format!("Downloaded {model}."))
                }
                OllamaOp::Delete => bridge.delete(&model).await.map(|_| format!("Deleted {model}.")),
                OllamaOp::Preload => bridge.preload(&model).await.map(|_| format!("{model} is loaded and ready.")),
            };
            notify.send(Event::OllamaOpDone(result));
        });
    }
}

#[derive(Clone, Copy)]
enum OllamaOp {
    Pull,
    Delete,
    Preload,
}

pub fn show(ui: &mut Ui, state: &mut SettingsState, chat_state: &mut ChatState, shared: &mut Shared) {
    let p = palette(ui);
    egui::CentralPanel::default().frame(Frame::new().fill(p.bg).inner_margin(Margin::same(16))).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let width = ui.available_width().min(980.0);
            ui.horizontal(|ui| {
                ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
                ui.vertical(|ui| {
                    ui.set_width(width);
                    theme::title(ui, "Settings", "Providers, local models, budget and data.");
                    ui.add_space(12.0);
                    providers(ui, state, shared);
                    ui.add_space(16.0);
                    theme::caption(ui, "Chat");
                    theme::card(ui, |ui| chat::options(ui, chat_state, shared));
                    ui.add_space(16.0);
                    ollama(ui, state, shared);
                    ui.add_space(16.0);
                    budget(ui, state, shared);
                    ui.add_space(16.0);
                    pricing_and_data(ui, shared);
                });
            });
        });
    });
}

fn providers(ui: &mut Ui, state: &mut SettingsState, shared: &mut Shared) {
    let p = palette(ui);
    theme::caption(ui, "Providers");
    theme::card(ui, |ui| {
        ui.label(
            RichText::new(
                "Keys are stored in your OS keychain (Windows Credential Manager, macOS Keychain or Secret Service). \
                 A <PROVIDER>_API_KEY environment variable is used when no key is saved.",
            )
            .small()
            .color(p.muted),
        );
        ui.add_space(4.0);
        let mut keys_changed = false;
        egui::Grid::new("providers_grid").num_columns(3).spacing([16.0, 10.0]).striped(true).show(ui, |ui| {
            for h in ["Provider", "API key", "Endpoint"] {
                theme::caption(ui, h);
            }
            ui.end_row();
            for prov in PROVIDERS {
                ui.vertical(|ui| {
                    ui.set_min_width(120.0);
                    ui.label(RichText::new(prov.short).strong());
                    let (text, color) = match shared.key_sources.get(prov.id).copied().flatten() {
                        Some(src) => (src, p.success),
                        None if !prov.needs_key => ("No key needed", p.muted),
                        None => ("No key", p.warning),
                    };
                    ui.label(RichText::new(text).small().color(color));
                });

                ui.horizontal(|ui| {
                    let input = state.key_inputs.entry(prov.id).or_default();
                    ui.add(
                        egui::TextEdit::singleline(input).password(true).hint_text("Paste key").desired_width(190.0),
                    );
                    if ui.add_enabled(!input.trim().is_empty(), egui::Button::new("Save")).clicked() {
                        match secrets::set_key(prov.id, input) {
                            Ok(()) => {
                                input.clear();
                                shared.notice(format!("{} key saved to the OS keychain.", prov.short), false);
                            }
                            Err(e) => shared.notice(format!("Could not save the {} key: {e}", prov.short), true),
                        }
                        keys_changed = true;
                    }
                    let saved = shared.key_sources.get(prov.id).copied().flatten() == Some("Saved in OS keychain");
                    if saved && ui.button("Remove").clicked() {
                        if let Err(e) = secrets::delete_key(prov.id) {
                            shared.notice(format!("Could not remove the {} key: {e}", prov.short), true);
                        }
                        keys_changed = true;
                    }
                });

                ui.horizontal(|ui| {
                    let url = state.url_inputs.entry(prov.id).or_default();
                    ui.add(egui::TextEdit::singleline(url).desired_width(280.0).font(egui::TextStyle::Monospace));
                    let current = shared.engine.base_url(prov.id);
                    if ui.add_enabled(*url != current, egui::Button::new("Save")).clicked() {
                        match shared.engine.set_base_url(prov.id, url) {
                            Ok(()) => {
                                if prov.id == "ollama" {
                                    shared.actions.push(Action::RefreshOllama);
                                }
                            }
                            Err(e) => shared.notice(format!("Could not save the endpoint: {e}"), true),
                        }
                    }
                    if current != prov.default_base_url && ui.small_button("Reset").clicked() {
                        *url = prov.default_base_url.to_string();
                        let _ = shared.engine.set_base_url(prov.id, prov.default_base_url);
                    }
                });
                ui.end_row();
            }
        });
        if keys_changed {
            shared.refresh_keys();
        }
    });
}

fn ollama(ui: &mut Ui, state: &mut SettingsState, shared: &mut Shared) {
    let p = palette(ui);
    theme::caption(ui, "Ollama models");
    theme::card(ui, |ui| {
        let status = shared.ollama.clone();
        ui.horizontal(|ui| {
            theme::dot(ui, if status.is_running { p.success } else { p.danger });
            ui.label(RichText::new(if status.is_running { "Online" } else { "Offline" }).strong());
            ui.label(RichText::new(&status.host).small().monospace().color(p.muted));
            if let Some(v) = &status.version {
                ui.label(RichText::new(format!("version {v}")).small().color(p.muted));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Rescan").clicked() {
                    shared.actions.push(Action::RefreshOllama);
                }
            });
        });
        if !status.is_running {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Install Ollama from ollama.com, then start it:").color(p.muted));
                ui.code("ollama serve");
            });
            return;
        }
        ui.separator();

        if status.available_models.is_empty() {
            ui.label(RichText::new("No models installed yet. Pull one below.").color(p.muted));
        } else {
            let mut op = None;
            egui::Grid::new("ollama_models").num_columns(5).striped(true).spacing([24.0, 6.0]).show(ui, |ui| {
                for h in ["Name", "Size", "Parameters", "Quantization", ""] {
                    theme::caption(ui, h);
                }
                ui.end_row();
                for m in &status.available_models {
                    ui.label(RichText::new(&m.name).strong());
                    ui.label(format!("{:.1} GB", m.size_gb));
                    ui.label(m.parameter_size.as_deref().unwrap_or("—"));
                    ui.label(m.quantization.as_deref().unwrap_or("—"));
                    ui.add_enabled_ui(!state.ollama_busy, |ui| {
                        ui.horizontal(|ui| {
                            if ui.small_button("Preload").on_hover_text("Load into memory now").clicked() {
                                op = Some((OllamaOp::Preload, m.name.clone()));
                            }
                            if ui.small_button(RichText::new("Delete").color(p.danger)).clicked() {
                                op = Some((OllamaOp::Delete, m.name.clone()));
                            }
                        });
                    });
                    ui.end_row();
                }
            });
            if let Some((o, name)) = op {
                state.ollama_op(shared, o, name);
            }
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut state.pull_name)
                    .hint_text("Model to download, e.g. llama3.2:3b")
                    .desired_width(280.0),
            );
            let ready = !state.pull_name.trim().is_empty() && !state.ollama_busy;
            if ui.add_enabled(ready, egui::Button::new("Pull")).clicked() {
                let name = state.pull_name.trim().to_string();
                state.pull_progress = Some(("starting".into(), None));
                state.ollama_op(shared, OllamaOp::Pull, name);
            }
            if state.ollama_busy {
                ui.spinner();
            }
        });
        if let Some((status, pct)) = &state.pull_progress {
            match pct {
                Some(pct) => {
                    ui.add(ProgressBar::new(*pct).text(format!("{status} · {:.0}%", pct * 100.0)).fill(p.accent))
                }
                None => ui.label(RichText::new(status).small().color(p.muted)),
            };
        }
    });
}

fn budget(ui: &mut Ui, state: &mut SettingsState, shared: &mut Shared) {
    let p = palette(ui);
    theme::caption(ui, "Monthly budget");
    theme::card(ui, |ui| {
        egui::Grid::new("budget_grid").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
            ui.label("Monthly cap");
            ui.add(
                egui::DragValue::new(&mut state.budget.monthly_cap_usd)
                    .prefix("$")
                    .range(0.01..=100_000.0)
                    .speed(0.5)
                    .fixed_decimals(2),
            );
            ui.end_row();
            ui.label("Warn at");
            ui.add(egui::Slider::new(&mut state.budget.warn_percent, 10.0..=100.0).suffix("%"));
            ui.end_row();
            ui.label("Hard stop");
            ui.checkbox(
                &mut state.budget.hard_stop,
                "Block cloud models once the cap is reached (local models keep working)",
            );
            ui.end_row();
        });
        ui.label(RichText::new(format!("Spent so far this month: ${:.4}", shared.month_spend)).small().color(p.muted));
    });
    if state.budget != shared.budget {
        match shared.engine.db.set_setting("budget", &state.budget) {
            Ok(()) => shared.actions.push(Action::DataChanged),
            Err(e) => shared.notice(format!("Could not save the budget: {e}"), true),
        }
        shared.budget = state.budget.clone();
    }
}

fn pricing_and_data(ui: &mut Ui, shared: &mut Shared) {
    let p = palette(ui);
    let engine: Arc<Engine> = Arc::clone(&shared.engine);
    theme::caption(ui, "Pricing");
    theme::card(ui, |ui| {
        let path = engine.pricing.read().unwrap_or_else(|e| e.into_inner()).path.clone();
        ui.label(
            RichText::new(
                "Prices are USD per million tokens. Providers change them, so check yours and edit the file.",
            )
            .small()
            .color(p.muted),
        );
        ui.horizontal(|ui| {
            ui.label(RichText::new(path.display().to_string()).monospace().small());
            if ui.button("Open file").clicked() {
                theme::open_path(&path);
            }
            if ui.button("Reload").clicked() {
                match engine.reload_pricing() {
                    Ok(()) => shared.notice("Pricing reloaded.", false),
                    Err(e) => shared.notice(e, true),
                }
            }
        });
    });

    ui.add_space(16.0);
    theme::caption(ui, "Data");
    theme::card(ui, |ui| {
        ui.label(
            RichText::new("History, the prompt library and settings live in one SQLite file in this folder.")
                .small()
                .color(p.muted),
        );
        ui.horizontal(|ui| {
            ui.label(RichText::new(engine.data_dir.display().to_string()).monospace().small());
            if ui.button("Open folder").clicked() {
                theme::open_path(&engine.data_dir);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_engine::local_bridge::LocalModelInfo;
    use crate::gui::test_support::{render_all, shared};

    #[test]
    fn renders_offline_and_online_ollama() {
        let (mut shared, _rx) = shared();
        let mut st = SettingsState::new(&shared.engine);
        let mut chat = ChatState::load(&shared.engine);
        render_all(|ui| show(ui, &mut st, &mut chat, &mut shared));
        shared.ollama.is_running = true;
        shared.ollama.available_models = vec![LocalModelInfo {
            name: "a-model-with-a-very-long-name:70b-instruct-q4_K_M".into(),
            size_gb: 39.5,
            parameter_size: Some("70B".into()),
            quantization: Some("Q4_K_M".into()),
        }];
        st.pull_progress = Some(("pulling".into(), Some(0.4)));
        st.ollama_busy = true;
        render_all(|ui| show(ui, &mut st, &mut chat, &mut shared));
    }

    #[test]
    fn pull_progress_and_completion_events() {
        let (mut shared, _rx) = shared();
        let mut st = SettingsState::new(&shared.engine);
        st.ollama_busy = true;
        st.handle(Event::PullProgress { status: "pulling".into(), pct: Some(0.5) }, &mut shared);
        assert_eq!(st.pull_progress, Some(("pulling".into(), Some(0.5))));
        st.handle(Event::OllamaOpDone(Err("model not found".into())), &mut shared);
        assert!(!st.ollama_busy && st.pull_progress.is_none());
        assert!(shared.notices.iter().any(|n| n.is_error && n.text == "model not found"));
        assert!(matches!(shared.actions.last(), Some(Action::RefreshOllama)));
    }
}
