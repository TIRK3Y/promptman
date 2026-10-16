use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Frame, Margin, RichText};
use egui_commonmark::CommonMarkCache;

use crate::core_engine::db::{ChatMessage, HistoryRow, ResultRecord};
use crate::core_engine::engine::{Budget, Engine};
use crate::core_engine::local_bridge::{LocalBridge, LocalBridgeStatus};
use crate::core_engine::providers::PROVIDERS;
use crate::core_engine::secrets;
use crate::gui::{analytics, batch, chat, compare, library, settings, theme};

#[derive(Clone, Copy, PartialEq)]
pub enum Tab {
    Chat,
    Compare,
    Batch,
    Library,
    Analytics,
    Settings,
}

const TABS: [(Tab, &str); 6] = [
    (Tab::Chat, "Chat"),
    (Tab::Compare, "Compare"),
    (Tab::Batch, "Batch"),
    (Tab::Library, "Library"),
    (Tab::Analytics, "Analytics"),
    (Tab::Settings, "Settings"),
];

/// Messages from background tasks to the UI thread.
pub enum Event {
    Delta {
        token: u64,
        slot: usize,
        text: String,
    },
    /// One model in a comparison finished; its record has no database id yet.
    SlotDone {
        token: u64,
        result: Box<ResultRecord>,
    },
    CompareDone {
        token: u64,
        run_id: Option<i64>,
        results: Vec<ResultRecord>,
        save_error: Option<String>,
    },
    JudgeDone {
        token: u64,
        scores: Result<Vec<(usize, f32, String)>, String>,
    },
    OptimizeDone {
        token: u64,
        result: Result<String, String>,
    },
    ChatDelta {
        token: u64,
        text: String,
    },
    ChatDone {
        token: u64,
        message: Box<ChatMessage>,
    },
    ChatOptimized {
        token: u64,
        result: Result<String, String>,
    },
    BatchCell {
        token: u64,
        row: usize,
        cell: Box<batch::Cell>,
    },
    BatchDone {
        token: u64,
    },
    Ollama(LocalBridgeStatus),
    PullProgress {
        status: String,
        pct: Option<f32>,
    },
    OllamaOpDone(Result<String, String>),
}

/// Sends an event and wakes the UI.
#[derive(Clone)]
pub struct Notifier {
    tx: Sender<Event>,
    ctx: egui::Context,
}

impl Notifier {
    pub fn send(&self, event: Event) {
        let _ = self.tx.send(event);
        self.ctx.request_repaint();
    }
}

/// Requests a tab can make of the rest of the app.
pub enum Action {
    OpenRun(i64),
    GoTo(Tab),
    RefreshOllama,
    DataChanged,
}

pub struct Notice {
    pub text: String,
    pub is_error: bool,
}

/// State every tab can read.
pub struct Shared {
    pub engine: Arc<Engine>,
    pub notify: Notifier,
    pub ollama: LocalBridgeStatus,
    pub history: Vec<HistoryRow>,
    pub month_spend: f64,
    pub budget: Budget,
    /// Bumped whenever stored runs change, so views can refresh cached queries.
    pub data_version: u64,
    pub key_sources: HashMap<&'static str, Option<&'static str>>,
    pub notices: Vec<Notice>,
    pub actions: Vec<Action>,
}

impl Shared {
    pub fn notice(&mut self, text: impl Into<String>, is_error: bool) {
        self.notices.push(Notice { text: text.into(), is_error });
    }

    pub fn refresh_keys(&mut self) {
        self.key_sources = PROVIDERS.iter().map(|p| (p.id, secrets::key_source(p.id))).collect();
    }

    fn refresh_data(&mut self) {
        self.history = self.engine.db.history(200).unwrap_or_default();
        self.month_spend = self.engine.month_spend();
        self.budget = self.engine.budget();
        self.data_version += 1;
    }
}

pub struct PromptManApp {
    shared: Shared,
    rx: Receiver<Event>,
    tab: Tab,
    chat: chat::ChatState,
    compare: compare::CompareState,
    batch: batch::BatchState,
    library: library::LibraryState,
    analytics: analytics::AnalyticsState,
    settings: settings::SettingsState,
    md_cache: CommonMarkCache,
    ollama_checked: Option<Instant>,
}

impl PromptManApp {
    pub fn new(cc: &eframe::CreationContext<'_>, engine: Engine, notes: Vec<String>) -> Self {
        Self::with_context(cc.egui_ctx.clone(), engine, notes)
    }

    /// Builds the app around an egui context (a real window's, or a headless one in tests).
    pub(crate) fn with_context(ctx: egui::Context, engine: Engine, notes: Vec<String>) -> Self {
        let (tx, rx) = channel();
        let engine = Arc::new(engine);
        let ollama_host = engine.base_url("ollama");
        let mut shared = Shared {
            engine: Arc::clone(&engine),
            notify: Notifier { tx, ctx },
            ollama: LocalBridgeStatus { is_running: false, host: ollama_host, version: None, available_models: vec![] },
            history: vec![],
            month_spend: 0.0,
            budget: Budget::default(),
            data_version: 0,
            key_sources: HashMap::new(),
            notices: vec![],
            actions: vec![],
        };
        for n in notes {
            let is_error = n.starts_with("Could not");
            shared.notice(n, is_error);
        }
        shared.refresh_data();
        shared.refresh_keys();

        // Reopen the tab the user last had open.
        let last_tab: Option<String> = engine.db.get_setting("ui.tab");
        let tab = TABS.iter().find(|(_, l)| last_tab.as_deref() == Some(*l)).map_or(Tab::Chat, |(t, _)| *t);

        Self {
            chat: chat::ChatState::load(&engine),
            compare: compare::CompareState::load(&engine),
            batch: batch::BatchState::default(),
            library: library::LibraryState::default(),
            analytics: analytics::AnalyticsState::default(),
            settings: settings::SettingsState::new(&engine),
            shared,
            rx,
            tab,
            md_cache: CommonMarkCache::default(),
            ollama_checked: None,
        }
    }

    fn check_ollama(&mut self) {
        self.ollama_checked = Some(Instant::now());
        let notify = self.shared.notify.clone();
        let host = self.shared.engine.base_url("ollama");
        tokio::spawn(async move {
            notify.send(Event::Ollama(LocalBridge::new(&host).check_status().await));
        });
    }

    fn handle_events(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Delta { .. }
                | Event::SlotDone { .. }
                | Event::CompareDone { .. }
                | Event::JudgeDone { .. }
                | Event::OptimizeDone { .. } => self.compare.handle(event, &mut self.shared),
                Event::ChatDelta { .. } | Event::ChatDone { .. } | Event::ChatOptimized { .. } => {
                    self.chat.handle(event, &mut self.shared)
                }
                Event::BatchCell { .. } | Event::BatchDone { .. } => self.batch.handle(event, &mut self.shared),
                Event::Ollama(status) => self.shared.ollama = status,
                Event::PullProgress { .. } | Event::OllamaOpDone(_) => self.settings.handle(event, &mut self.shared),
            }
        }
    }

    fn run_actions(&mut self) {
        for action in std::mem::take(&mut self.shared.actions) {
            match action {
                Action::OpenRun(id) => {
                    self.compare.open_run(id, &mut self.shared);
                    self.set_tab(Tab::Compare);
                }
                Action::GoTo(tab) => self.set_tab(tab),
                Action::RefreshOllama => self.check_ollama(),
                Action::DataChanged => self.shared.refresh_data(),
            }
        }
    }

    fn set_tab(&mut self, tab: Tab) {
        if self.tab != tab {
            self.tab = tab;
            if let Some((_, label)) = TABS.iter().find(|(t, _)| *t == tab) {
                let _ = self.shared.engine.db.set_setting("ui.tab", &label.to_string());
            }
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui);
        let mut go_to = None;
        egui::Panel::top("top_bar")
            .exact_size(52.0)
            .frame(Frame::new().fill(p.surface).inner_margin(Margin::symmetric(16, 0)))
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    theme::logo(ui);
                    ui.label(RichText::new("PromptMan").size(16.0).strong());
                    ui.add_space(20.0);
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (tab, label) in TABS {
                        if theme::nav_tab(ui, self.tab == tab, label).clicked() {
                            go_to = Some(tab);
                        }
                    }
                    ui.spacing_mut().item_spacing.x = 8.0;

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        egui::widgets::global_theme_preference_switch(ui);

                        let s = &self.shared;
                        let ratio = s.month_spend / s.budget.monthly_cap_usd.max(0.0001);
                        let color = theme::budget_color(p, ratio, s.budget.warn_percent);
                        // Right-to-left row: label first so the dot ends up on its left.
                        let budget = theme::chip(ui, |ui| {
                            ui.label(
                                RichText::new(format!("${:.2} of ${:.0}", s.month_spend, s.budget.monthly_cap_usd))
                                    .small()
                                    .color(p.text),
                            );
                            theme::dot(ui, color);
                        })
                        .interact(egui::Sense::click())
                        .on_hover_text(format!(
                            "Spent this month: ${:.4} of your ${:.2} budget. Click to change it.",
                            s.month_spend, s.budget.monthly_cap_usd
                        ))
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                        if budget.clicked() {
                            go_to = Some(Tab::Settings);
                        }

                        let (color, text) = if s.ollama.is_running {
                            (p.success, format!("Ollama · {} models", s.ollama.available_models.len()))
                        } else {
                            (p.muted, "Ollama offline".to_string())
                        };
                        theme::chip(ui, |ui| {
                            ui.label(RichText::new(text).small().color(p.text));
                            theme::dot(ui, color);
                        });
                    });
                });
            });
        if let Some(tab) = go_to {
            self.set_tab(tab);
        }
    }

    fn notices(&mut self, ui: &mut egui::Ui) {
        if self.shared.notices.is_empty() {
            return;
        }
        let p = theme::palette(ui);
        let mut dismiss = None;
        egui::Panel::top("notices")
            .frame(Frame::new().fill(p.bg).inner_margin(Margin::symmetric(16, 8)))
            .show_separator_line(false)
            .show(ui, |ui| {
                for (i, n) in self.shared.notices.iter().enumerate() {
                    let color = if n.is_error { p.danger } else { p.accent };
                    theme::banner(ui, color, |ui| {
                        ui.label(RichText::new(&n.text).color(p.text));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Dismiss").clicked() {
                                dismiss = Some(i);
                            }
                        });
                    });
                }
            });
        if let Some(i) = dismiss {
            self.shared.notices.remove(i);
        }
    }
}

impl eframe::App for PromptManApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
    }
}

impl PromptManApp {
    /// One frame of the whole app.
    fn frame(&mut self, ui: &mut egui::Ui) {
        self.handle_events();
        if self.ollama_checked.is_none_or(|t| t.elapsed() > Duration::from_secs(15)) {
            self.check_ollama();
        }

        self.top_bar(ui);
        self.notices(ui);

        match self.tab {
            Tab::Chat => chat::show(ui, &mut self.chat, &mut self.shared, &mut self.md_cache),
            Tab::Compare => compare::show(ui, &mut self.compare, &mut self.shared, &mut self.md_cache),
            Tab::Batch => batch::show(ui, &mut self.batch, &self.compare, &mut self.shared),
            Tab::Library => library::show(ui, &mut self.library, &mut self.compare, &mut self.shared),
            Tab::Analytics => analytics::show(ui, &mut self.analytics, &mut self.shared),
            Tab::Settings => settings::show(ui, &mut self.settings, &mut self.chat, &mut self.shared),
        }

        self.run_actions();
        // Keeps the Ollama status fresh while idle.
        ui.ctx().request_repaint_after(Duration::from_secs(15));
    }
}

#[cfg(test)]
impl Shared {
    /// State for GUI tests: no window, no keychain reads, events go to the returned receiver.
    pub(crate) fn for_test(engine: Engine) -> (Self, Receiver<Event>) {
        let (tx, rx) = channel();
        let host = engine.base_url("ollama");
        let shared = Self {
            engine: Arc::new(engine),
            notify: Notifier { tx, ctx: egui::Context::default() },
            ollama: LocalBridgeStatus { is_running: false, host, version: None, available_models: vec![] },
            history: vec![],
            month_spend: 0.0,
            budget: Budget::default(),
            data_version: 0,
            key_sources: HashMap::new(),
            notices: vec![],
            actions: vec![],
        };
        (shared, rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_engine::db::{ChatMessage, RunRecord};
    use crate::core_engine::llm::Role;
    use crate::core_engine::local_bridge::LocalModelInfo;
    use crate::core_engine::providers::ModelRef;
    use crate::gui::test_support::{render_frames, NOWHERE, SIZES};

    /// A database with something to show on every screen.
    fn sample_engine() -> Engine {
        let engine = Engine::for_test(NOWHERE);
        let db = &engine.db;
        let answer = |provider: &str, model: &str, output: &str, cost: Option<f64>| ResultRecord {
            provider: provider.into(),
            model: model.into(),
            output: output.into(),
            cost_usd: cost,
            latency_ms: 812.0,
            ttft_ms: Some(120.0),
            prompt_tokens: 40,
            completion_tokens: 120,
            tokens_per_sec: 33.0,
            is_local: provider == "ollama",
            rating: Some(4),
            ..Default::default()
        };
        let failed = ResultRecord {
            error: Some("No API key for Anthropic. Add one in Settings.".into()),
            ..answer("anthropic", "claude-opus-5-5", "", None)
        };
        let markdown = "# Threads\n\n- share memory\n- | a | b |\n\n```rust\nfn main() {}\n```";
        db.save_run(
            &RunRecord { kind: "compare".into(), prompt: "Explain threads".into(), ..Default::default() },
            &[
                answer("openai", "gpt-4o", markdown, Some(0.0012)),
                answer("ollama", "llama3", "Short.", Some(0.0)),
                failed,
            ],
        )
        .unwrap();
        db.save_run(
            &RunRecord { kind: "optimize".into(), prompt: "<draft>\nhey\n</draft>".into(), ..Default::default() },
            &[answer("anthropic", "claude-opus-5-5", "Say hi.", Some(0.0001))],
        )
        .unwrap();
        let conv = db
            .create_conversation(
                "A chat whose title is long enough to be truncated in the list",
                &ModelRef::new("ollama", "llama3"),
            )
            .unwrap();
        db.add_message(conv, &ChatMessage { content: "Hello".into(), ..Default::default() }).unwrap();
        let reply = ChatMessage {
            role: Role::Assistant,
            content: "Hi! **bold** and `code`".into(),
            model: Some(ModelRef::new("ollama", "llama3")),
            ..Default::default()
        };
        db.add_message(conv, &reply).unwrap();
        db.save_prompt("Summarizer", "writing, short", "Be brief.", "Summarize {{text}}").unwrap();
        db.save_prompt("Summarizer", "writing, short", "Be brief.", "Summarize {{text}} in 3 bullets").unwrap();
        // Over the cap with the hard stop on: the warning states render too.
        db.set_setting("budget", &Budget { monthly_cap_usd: 0.001, warn_percent: 80.0, hard_stop: true }).unwrap();
        engine
    }

    fn app(ctx: &egui::Context) -> PromptManApp {
        let notes = vec!["Could not read prices (test).".to_string(), "Imported 2 records.".to_string()];
        let mut app = PromptManApp::with_context(ctx.clone(), sample_engine(), notes);
        app.ollama_checked = Some(Instant::now()); // keep the test off the network
        app.shared.ollama.is_running = true;
        app.shared.ollama.version = Some("0.9.1".into());
        app.shared.ollama.available_models = vec![LocalModelInfo {
            name: "llama3:8b".into(),
            size_gb: 4.7,
            parameter_size: Some("8B".into()),
            quantization: Some("Q4_0".into()),
        }];
        app
    }

    #[test]
    fn every_tab_renders_at_every_size_in_both_themes() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut app = app(&ctx);
        assert_eq!(app.shared.notices.len(), 2);
        assert!(app.shared.notices[0].is_error && !app.shared.notices[1].is_error);
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            ctx.set_theme(theme);
            for (w, h) in SIZES {
                for (tab, _) in TABS {
                    app.tab = tab;
                    render_frames(&ctx, egui::vec2(w, h), &mut |ui| app.frame(ui));
                }
            }
        }
        assert!(app.shared.actions.is_empty(), "rendering alone must not trigger actions");
    }

    #[test]
    fn a_saved_run_opens_in_compare() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.tab = Tab::Chat;
        let run_id = app.shared.history.iter().find(|h| h.kind == "compare").unwrap().run_id;
        app.shared.actions.push(Action::OpenRun(run_id));
        app.run_actions();
        assert!(app.tab == Tab::Compare);
        assert_eq!(app.compare.slots.len(), 3);
        assert!(app.compare.viewing_past.is_some());
        render_frames(&ctx, egui::vec2(1280.0, 800.0), &mut |ui| app.frame(ui));
    }

    #[test]
    fn last_tab_is_remembered() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.set_tab(Tab::Library);
        assert_eq!(app.shared.engine.db.get_setting::<String>("ui.tab").as_deref(), Some("Library"));
    }

    #[test]
    fn data_changes_refresh_spend_and_history() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let before = (app.shared.history.len(), app.shared.data_version);
        app.shared
            .engine
            .db
            .save_run(&RunRecord { kind: "cli".into(), ..Default::default() }, &[ResultRecord::default()])
            .unwrap();
        app.shared.actions.push(Action::DataChanged);
        app.run_actions();
        assert_eq!(app.shared.history.len(), before.0 + 1);
        assert!(app.shared.data_version > before.1);
        assert!(app.shared.month_spend > 0.0);
    }
}
