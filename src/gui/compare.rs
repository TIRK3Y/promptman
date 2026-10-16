//! Compare tab: state, run orchestration, and the three-pane layout.
use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui::{self, Frame, Label, Margin, ProgressBar, RichText, Sense, Ui};
use egui_commonmark::CommonMarkCache;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::theme::{self, palette};
use super::{composer, results};
use crate::app::{Action, Event, Shared};
use crate::core_engine::db::{ResultRecord, RunRecord};
use crate::core_engine::engine::{Engine, RunHooks};
use crate::core_engine::evals::{fill_template, judge_prompt, parse_judge, JUDGE_SYSTEM};
use crate::core_engine::llm::{Message, Params, Prompt};
use crate::core_engine::providers::ModelRef;
use crate::core_engine::token_auditor::{audit_prompt, TokenMetrics};

pub const MAX_TARGETS: usize = 4;

pub struct Slot {
    pub target: ModelRef,
    pub text: String,
    pub result: Option<ResultRecord>,
}

/// A finished exchange in conversation mode: one user message, one reply per target.
pub struct Turn {
    pub user: String,
    pub replies: Vec<String>,
}

/// Composer fields remembered between sessions.
#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    targets: Vec<ModelRef>,
    system: String,
    prompt: String,
    params: Params,
    judge: ModelRef,
    optimizer: ModelRef,
}

impl Default for Saved {
    fn default() -> Self {
        Self {
            targets: vec![ModelRef::new("openai", "gpt-4o"), ModelRef::new("ollama", "llama3")],
            system: String::new(),
            prompt: "Explain the difference between a process and a thread in three sentences.".into(),
            params: Params::default(),
            judge: ModelRef::new("anthropic", "claude-opus-5-5"),
            optimizer: default_optimizer(),
        }
    }
}

fn default_optimizer() -> ModelRef {
    ModelRef::new("anthropic", "claude-opus-5-5")
}

pub struct CompareState {
    pub targets: Vec<ModelRef>,
    pub system: String,
    pub prompt: String,
    pub vars: HashMap<String, String>,
    pub params: Params,
    pub chat_mode: bool,
    pub turns: Vec<Turn>,
    /// The latest turn's replies are already in `turns`, so cards shouldn't repeat them.
    pub slots_in_turns: bool,
    pub slots: Vec<Slot>,
    /// Prompt that produced the results on screen.
    pub shown_prompt: String,
    /// System prompt and parameters of the run on screen (the Composer may have changed since).
    pub shown_system: String,
    pub shown_params: Params,
    pub run_id: Option<i64>,
    /// Set when the results come from history: the run's timestamp.
    pub viewing_past: Option<String>,
    pub running: bool,
    pub token: u64,
    /// Stops the whole run; each slot's token is a child of it.
    run_cancel: Option<CancellationToken>,
    slot_cancels: Vec<CancellationToken>,
    pub show_diff: bool,
    pub markdown: bool,
    pub diff_pair: (usize, usize),
    pub judge_model: ModelRef,
    pub judging: bool,
    pub judge_error: Option<String>,
    /// Model that rewrites drafts into clearer, shorter prompts.
    pub optimizer_model: ModelRef,
    pub optimizing: bool,
    /// Suggested rewrite awaiting the user's decision.
    pub optimized: Option<String>,
    pub optimize_error: Option<String>,
    optimize_token: u64,
    /// JSON of the Composer as last saved, to skip writes when nothing changed.
    last_saved: String,
    metrics_key: (String, String),
    metrics: TokenMetrics,
}

impl CompareState {
    pub fn load(engine: &Engine) -> Self {
        let saved: Option<Saved> = engine.db.get_setting("composer");
        let saved = saved.unwrap_or_default();
        Self {
            targets: saved.targets,
            system: saved.system,
            prompt: saved.prompt,
            vars: HashMap::new(),
            params: saved.params,
            chat_mode: false,
            turns: vec![],
            slots_in_turns: false,
            slots: vec![],
            shown_prompt: String::new(),
            shown_system: String::new(),
            shown_params: Params::default(),
            run_id: None,
            viewing_past: None,
            running: false,
            token: 0,
            run_cancel: None,
            slot_cancels: vec![],
            show_diff: false,
            markdown: true,
            diff_pair: (0, 1),
            judge_model: saved.judge,
            judging: false,
            judge_error: None,
            optimizer_model: saved.optimizer,
            optimizing: false,
            optimized: None,
            optimize_error: None,
            optimize_token: 0,
            last_saved: String::new(),
            metrics_key: Default::default(),
            metrics: TokenMetrics::default(),
        }
    }

    /// Saves the Composer if anything in it changed since the last save.
    pub fn persist_if_changed(&mut self, engine: &Engine) {
        let saved = Saved {
            targets: self.targets.clone(),
            system: self.system.clone(),
            prompt: self.prompt.clone(),
            params: self.params.clone(),
            judge: self.judge_model.clone(),
            optimizer: self.optimizer_model.clone(),
        };
        let json = serde_json::to_string(&saved).unwrap_or_default();
        if json != self.last_saved {
            let _ = engine.db.set_setting("composer", &saved);
            self.last_saved = json;
        }
    }

    fn persist(&self, engine: &Engine) {
        let saved = Saved {
            targets: self.targets.clone(),
            system: self.system.clone(),
            prompt: self.prompt.clone(),
            params: self.params.clone(),
            judge: self.judge_model.clone(),
            optimizer: self.optimizer_model.clone(),
        };
        let _ = engine.db.set_setting("composer", &saved);
    }

    pub fn filled_prompt(&self) -> String {
        fill_template(&self.prompt, &self.vars)
    }

    /// Token metrics for the prompt, recomputed only when it changes.
    pub fn metrics(&mut self) -> &TokenMetrics {
        let text = format!("{}\n{}", self.system, self.filled_prompt());
        let model = self.targets.first().map(|t| t.model.clone()).unwrap_or_default();
        if self.metrics_key != (text.clone(), model.clone()) {
            self.metrics = audit_prompt(&text, &model);
            self.metrics_key = (text, model);
        }
        &self.metrics
    }

    /// Why a run can't start right now, if it can't.
    pub fn blocked_reason(&self, shared: &Shared) -> Option<String> {
        if self.running {
            Some("A run is in progress".into())
        } else if self.prompt.trim().is_empty() {
            Some("Write a prompt first".into())
        } else if self.targets.iter().any(|t| t.model.trim().is_empty()) {
            Some("Choose a model for every target".into())
        } else if self.targets.iter().any(|t| !t.is_local()) && shared.engine.cloud_blocked() {
            Some("Monthly budget reached; cloud models are paused (see Settings)".into())
        } else {
            None
        }
    }

    pub fn new_chat(&mut self) {
        self.turns.clear();
        self.slots_in_turns = false;
    }

    pub fn start_run(&mut self, shared: &mut Shared) {
        if self.blocked_reason(shared).is_some() {
            return;
        }
        self.persist(&shared.engine);
        let prompt = self.filled_prompt();
        let conversations: Vec<Vec<Message>> = (0..self.targets.len())
            .map(|i| {
                let mut msgs = Vec::new();
                if self.chat_mode {
                    for t in &self.turns {
                        msgs.push(Message::user(t.user.clone()));
                        let reply = t.replies.get(i).filter(|r| !r.trim().is_empty());
                        msgs.push(Message::assistant(reply.cloned().unwrap_or_else(|| "(no response)".into())));
                    }
                }
                msgs.push(Message::user(prompt.clone()));
                msgs
            })
            .collect();
        let run = RunRecord {
            kind: "compare".into(),
            prompt: prompt.clone(),
            system: self.system.clone(),
            params: self.params.clone(),
            history: if self.chat_mode {
                self.turns.iter().map(|t| Message::user(t.user.clone())).collect()
            } else {
                vec![]
            },
            ..Default::default()
        };

        self.token += 1;
        self.running = true;
        self.slots_in_turns = false;
        self.viewing_past = None;
        self.run_id = None;
        self.judge_error = None;
        self.shown_prompt = prompt;
        self.shown_system = self.system.clone();
        self.shown_params = self.params.clone();
        self.slots =
            self.targets.iter().map(|t| Slot { target: t.clone(), text: String::new(), result: None }).collect();
        self.diff_pair = (0, 1.min(self.slots.len().saturating_sub(1)));

        let run_cancel = CancellationToken::new();
        self.slot_cancels = self.targets.iter().map(|_| run_cancel.child_token()).collect();
        self.run_cancel = Some(run_cancel);
        let cancels = self.slot_cancels.clone();

        let engine = Arc::clone(&shared.engine);
        let notify = shared.notify.clone();
        let token = self.token;
        let targets = self.targets.clone();
        let system = self.system.clone();
        let params = self.params.clone();
        tokio::spawn(async move {
            let (stream, done) = (notify.clone(), notify.clone());
            let hooks = RunHooks {
                on_delta: Arc::new(move |slot: usize, d: &str| {
                    stream.send(Event::Delta { token, slot, text: d.to_string() })
                }),
                on_done: Arc::new(move |r: &ResultRecord| {
                    done.send(Event::SlotDone { token, result: Box::new(r.clone()) })
                }),
            };
            let mut results = engine.run_all(&targets, &system, conversations, &params, cancels, hooks).await;
            let (run_id, save_error) = match engine.db.save_run(&run, &results) {
                Ok((id, ids)) => {
                    results.iter_mut().zip(ids).for_each(|(r, id)| r.id = Some(id));
                    (Some(id), None)
                }
                Err(e) => (None, Some(e)),
            };
            notify.send(Event::CompareDone { token, run_id, results, save_error });
        });
    }

    /// Stops every model in the running comparison; partial answers are kept.
    pub fn stop_all(&mut self) {
        if let Some(c) = &self.run_cancel {
            c.cancel();
        }
    }

    /// Stops one model; the others keep going.
    pub fn stop_slot(&mut self, slot: usize) {
        if let Some(c) = self.slot_cancels.get(slot) {
            c.cancel();
        }
    }

    pub fn stopping_all(&self) -> bool {
        self.run_cancel.as_ref().is_some_and(|c| c.is_cancelled())
    }

    pub fn slot_stopping(&self, slot: usize) -> bool {
        self.slot_cancels.get(slot).is_some_and(|c| c.is_cancelled())
    }

    pub fn start_judge(&mut self, shared: &mut Shared) {
        let answers: Vec<(usize, String)> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                s.result
                    .as_ref()
                    .filter(|r| r.error.is_none() && !r.output.trim().is_empty())
                    .map(|r| (i, r.output.clone()))
            })
            .collect();
        if answers.is_empty() || self.judging {
            return;
        }
        self.persist(&shared.engine);
        self.judging = true;
        self.judge_error = None;

        let texts: Vec<&str> = answers.iter().map(|(_, t)| t.as_str()).collect();
        let request = judge_prompt(&self.shown_prompt, &texts);
        let engine = Arc::clone(&shared.engine);
        let notify = shared.notify.clone();
        let judge = self.judge_model.clone();
        let token = self.token;
        tokio::spawn(async move {
            // Room for thinking on models that always think.
            let params = Params { temperature: 0.0, top_p: 1.0, max_tokens: 8000 };
            let never = CancellationToken::new();
            let messages = [Message::user(request.clone())];
            let rec = engine
                .run_one(
                    0,
                    &judge,
                    Prompt { system: JUDGE_SYSTEM, messages: &messages, params: &params },
                    &never,
                    |_| {},
                )
                .await;
            let run = RunRecord {
                kind: "judge".into(),
                prompt: request,
                system: JUDGE_SYSTEM.into(),
                params,
                ..Default::default()
            };
            let _ = engine.db.save_run(&run, std::slice::from_ref(&rec));
            let scores = match rec.error {
                Some(e) => Err(e),
                None => parse_judge(&rec.output, answers.len()).map(|scores| {
                    answers.iter().zip(scores).map(|((slot, _), (score, reason))| (*slot, score, reason)).collect()
                }),
            };
            notify.send(Event::JudgeDone { token, scores });
        });
    }

    /// Asks the optimizer model for a clearer, shorter version of the prompt.
    pub fn start_optimize(&mut self, shared: &mut Shared) {
        if self.optimizing || self.prompt.trim().is_empty() {
            return;
        }
        self.persist(&shared.engine);
        self.optimizing = true;
        self.optimized = None;
        self.optimize_error = None;
        self.optimize_token += 1;

        let engine = Arc::clone(&shared.engine);
        let notify = shared.notify.clone();
        let model = self.optimizer_model.clone();
        let draft = self.prompt.clone();
        let token = self.optimize_token;
        tokio::spawn(async move {
            let result = engine.optimize_prompt(&model, &draft).await;
            notify.send(Event::OptimizeDone { token, result });
        });
    }

    pub fn handle(&mut self, event: Event, shared: &mut Shared) {
        match event {
            Event::OptimizeDone { token, result } if token == self.optimize_token => {
                self.optimizing = false;
                match result {
                    Ok(text) => self.optimized = Some(text),
                    Err(e) => self.optimize_error = Some(e),
                }
                shared.actions.push(Action::DataChanged);
            }
            Event::Delta { token, slot, text } if token == self.token => {
                if let Some(s) = self.slots.get_mut(slot) {
                    s.text.push_str(&text);
                }
            }
            // Show each model's result as soon as it is ready, not when the slowest one finishes.
            Event::SlotDone { token, result } if token == self.token => {
                if let Some(s) = self.slots.get_mut(result.slot).filter(|s| s.result.is_none()) {
                    s.text = result.output.clone();
                    s.result = Some(*result);
                }
            }
            Event::CompareDone { token, run_id, results, save_error } if token == self.token => {
                self.running = false;
                self.run_cancel = None;
                self.slot_cancels.clear();
                self.run_id = run_id;
                for r in results {
                    if let Some(s) = self.slots.get_mut(r.slot) {
                        s.text = r.output.clone();
                        s.result = Some(r);
                    }
                }
                if self.chat_mode {
                    let replies = self.slots.iter().map(|s| s.text.clone()).collect();
                    self.turns.push(Turn { user: self.shown_prompt.clone(), replies });
                    self.slots_in_turns = true;
                    self.prompt.clear();
                }
                if let Some(e) = save_error {
                    shared.notice(format!("The run finished but could not be saved: {e}"), true);
                }
                shared.actions.push(Action::DataChanged);
            }
            Event::JudgeDone { token, scores } => {
                self.judging = false;
                if token != self.token {
                    return;
                }
                match scores {
                    Ok(scores) => {
                        for (slot, score, reason) in scores {
                            if let Some(r) = self.slots.get_mut(slot).and_then(|s| s.result.as_mut()) {
                                r.judge_score = Some(score);
                                r.judge_reason = Some(reason.clone());
                                if let Some(id) = r.id {
                                    let _ = shared.engine.db.set_judge(id, score, &reason);
                                }
                            }
                        }
                    }
                    Err(e) => self.judge_error = Some(e),
                }
                shared.actions.push(Action::DataChanged);
            }
            _ => {}
        }
    }

    /// Shows a stored run in the results pane.
    pub fn open_run(&mut self, run_id: i64, shared: &mut Shared) {
        match shared.engine.db.load_run(run_id) {
            Ok((run, results)) => {
                // Don't leave a run billing in the background: stop it (it still saves what it got).
                self.stop_all();
                self.run_cancel = None;
                self.slot_cancels.clear();
                self.token += 1;
                self.running = false;
                self.judging = false;
                self.judge_error = None;
                self.slots_in_turns = false;
                self.shown_prompt = run.prompt;
                self.shown_system = run.system;
                self.shown_params = run.params;
                self.run_id = run.id;
                self.viewing_past = Some(run.created_at);
                self.slots = results
                    .into_iter()
                    .map(|r| Slot {
                        target: ModelRef::new(&r.provider, &r.model),
                        text: r.output.clone(),
                        result: Some(r),
                    })
                    .collect();
                self.diff_pair = (0, 1.min(self.slots.len().saturating_sub(1)));
            }
            Err(e) => shared.notice(format!("Could not open that run: {e}"), true),
        }
    }

    pub fn set_rating(&mut self, slot: usize, rating: Option<u8>, shared: &mut Shared) {
        if let Some(r) = self.slots.get_mut(slot).and_then(|s| s.result.as_mut()) {
            r.rating = rating;
            if let Some(id) = r.id {
                if let Err(e) = shared.engine.db.set_rating(id, rating) {
                    shared.notice(format!("Could not save the rating: {e}"), true);
                }
            }
            shared.actions.push(Action::DataChanged);
        }
    }
}

pub fn show(ui: &mut Ui, state: &mut CompareState, shared: &mut Shared, md_cache: &mut CommonMarkCache) {
    state.persist_if_changed(&shared.engine);
    let p = palette(ui);
    let body = Frame::new().fill(p.bg).inner_margin(Margin::same(16));
    // Caps keep the results pane visible however far the side panels are dragged.
    let total = ui.available_width();

    egui::Panel::left("composer")
        .resizable(true)
        .default_size(400.0)
        .size_range(320.0..=(total * 0.4).max(320.0))
        .frame(body)
        .show(ui, |ui| theme::contained(ui, |ui| composer::show(ui, state, shared)));

    egui::Panel::right("usage")
        .resizable(true)
        .default_size(300.0)
        .size_range(260.0..=(total * 0.3).max(260.0))
        .frame(body)
        .show(ui, |ui| theme::contained(ui, |ui| usage(ui, state, shared)));

    egui::CentralPanel::default().frame(body).show(ui, |ui| results::show(ui, state, shared, md_cache));
}

fn usage(ui: &mut Ui, state: &mut CompareState, shared: &mut Shared) {
    let p = palette(ui);
    theme::title(ui, "Usage", "Prompt size, spend and history.");
    ui.add_space(8.0);

    let prompt_tokens = state.metrics().token_count;
    let encoding = state.metrics().bpe_encoding;
    let input_cost: Option<f64> = {
        let pricing = shared.engine.pricing.read().unwrap_or_else(|e| e.into_inner());
        state.targets.iter().map(|t| pricing.cost(t, prompt_tokens, 0)).sum()
    };
    theme::card(ui, |ui| {
        ui.columns(2, |cols| {
            theme::stat(&mut cols[0], "Prompt tokens", prompt_tokens.to_string());
            let cost = match input_cost {
                Some(0.0) => "Free".to_string(),
                Some(c) => format!("${c:.5}"),
                None => "Unknown".to_string(),
            };
            theme::stat(&mut cols[1], "Input cost", cost);
        });
        ui.label(
            RichText::new(format!("{encoding} · input only, across {} model(s)", state.targets.len()))
                .small()
                .color(p.muted),
        );
    });

    ui.add_space(8.0);
    theme::caption(ui, "Budget this month");
    let b = &shared.budget;
    let ratio = shared.month_spend / b.monthly_cap_usd.max(0.0001);
    let color = theme::budget_color(p, ratio, b.warn_percent);
    theme::card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("${:.4}", shared.month_spend)).strong());
            ui.label(RichText::new(format!("of ${:.2}", b.monthly_cap_usd)).color(p.muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(format!("{:.0}%", ratio * 100.0)).small().color(color));
            });
        });
        ui.add(ProgressBar::new(ratio.min(1.0) as f32).desired_height(6.0).fill(color));
        if ratio >= 1.0 {
            let msg = if b.hard_stop { "Cap reached: cloud runs are paused." } else { "Cap reached." };
            ui.label(RichText::new(msg).small().color(p.danger));
        } else if ratio * 100.0 >= b.warn_percent as f64 {
            ui.label(RichText::new("Approaching this month's cap.").small().color(p.warning));
        }
    });

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        theme::caption(ui, "History");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new("Click a run to open it").small().color(p.muted));
        });
    });
    theme::card(ui, |ui| {
        if shared.history.is_empty() {
            ui.label(RichText::new("No runs yet.").color(p.muted));
            return;
        }
        egui::ScrollArea::vertical().id_salt("history").auto_shrink(false).show(ui, |ui| {
            let mut i = 0;
            let rows = &shared.history;
            while i < rows.len() {
                let run_id = rows[i].run_id;
                let end = rows[i..].iter().position(|r| r.run_id != run_id).map_or(rows.len(), |n| i + n);
                if i > 0 {
                    ui.separator();
                }
                let resp = ui
                    .vertical(|ui| {
                        let preview = rows[i].prompt.replace("<draft>", "").replace("</draft>", "").replace('\n', " ");
                        let label = match rows[i].kind.as_str() {
                            "optimize" => format!("Prompt rewrite · {}", preview.trim()),
                            "judge" => "Judge scoring".to_string(),
                            "batch" => format!("Batch · {}", preview.trim()),
                            "cli" => format!("CLI · {}", preview.trim()),
                            "chat" => format!("Chat · {}", preview.trim()),
                            _ => preview.trim().to_string(),
                        };
                        ui.add(Label::new(RichText::new(label).strong()).truncate());
                        ui.label(RichText::new(&rows[i].created_at).small().color(p.muted));
                        for r in &rows[i..end] {
                            // Right-to-left so the cost keeps its space and the model name truncates.
                            let row = egui::vec2(ui.available_width(), 18.0);
                            ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let cost = match r.cost_usd {
                                    _ if r.failed => "failed".to_string(),
                                    Some(0.0) => "Free".to_string(),
                                    Some(c) => format!("${c:.5}"),
                                    None => "?".to_string(),
                                };
                                ui.label(RichText::new(cost).small().monospace().color(p.muted));
                                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    theme::dot(
                                        ui,
                                        if r.failed {
                                            p.danger
                                        } else if r.is_local {
                                            p.success
                                        } else {
                                            p.accent
                                        },
                                    );
                                    ui.add(Label::new(RichText::new(&r.model).small()).truncate());
                                });
                            });
                        }
                    })
                    .response
                    .interact(Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if resp.clicked() {
                    shared.actions.push(Action::OpenRun(run_id));
                }
                i = end;
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_engine::engine::Budget;
    use crate::gui::test_support::{next_event, render_all, shared};

    fn result(slot: usize, output: &str) -> ResultRecord {
        ResultRecord {
            slot,
            provider: "custom".into(),
            model: format!("m{slot}"),
            output: output.into(),
            cost_usd: Some(0.0),
            latency_ms: 900.0,
            ..Default::default()
        }
    }

    fn running(shared: &Shared, n: usize) -> CompareState {
        let mut st = CompareState::load(&shared.engine);
        st.targets = (0..n).map(|i| ModelRef::new("custom", &format!("m{i}"))).collect();
        st.slots = st.targets.iter().map(|t| Slot { target: t.clone(), text: String::new(), result: None }).collect();
        st.running = true;
        st.token = 7;
        let parent = CancellationToken::new();
        st.slot_cancels = (0..n).map(|_| parent.child_token()).collect();
        st.run_cancel = Some(parent);
        st
    }

    #[test]
    fn each_model_shows_its_result_as_soon_as_it_finishes() {
        let (mut shared, _rx) = shared();
        let mut st = running(&shared, 2);
        st.handle(Event::Delta { token: 7, slot: 0, text: "stream".into() }, &mut shared);
        st.handle(Event::SlotDone { token: 7, result: Box::new(result(1, "second")) }, &mut shared);
        assert!(st.slots[0].result.is_none() && st.slots[1].result.is_some());
        assert_eq!(st.slots[0].text, "stream");
        st.handle(Event::SlotDone { token: 6, result: Box::new(result(0, "stale")) }, &mut shared);
        assert!(st.slots[0].result.is_none(), "results from an older run are ignored");

        let mut saved = vec![result(0, "first"), result(1, "second")];
        saved.iter_mut().enumerate().for_each(|(i, r)| r.id = Some(i as i64 + 10));
        st.handle(Event::CompareDone { token: 7, run_id: Some(3), results: saved, save_error: None }, &mut shared);
        assert!(!st.running && st.run_cancel.is_none() && st.slot_cancels.is_empty());
        assert_eq!(st.run_id, Some(3));
        assert!(st.slots.iter().all(|s| s.result.as_ref().unwrap().id.is_some()), "ids arrive with CompareDone");
        assert!(matches!(shared.actions.last(), Some(Action::DataChanged)));
    }

    #[test]
    fn stopping_one_model_or_all() {
        let (_shared, _rx) = shared();
        let mut st = running(&_shared, 3);
        st.stop_slot(1);
        assert!(st.slot_stopping(1) && !st.slot_stopping(0) && !st.stopping_all());
        st.stop_slot(9); // out of range: ignored
        st.stop_all();
        assert!(st.stopping_all() && (0..3).all(|i| st.slot_stopping(i)));
    }

    #[test]
    fn a_save_error_is_shown_as_a_notice() {
        let (mut shared, _rx) = shared();
        let mut st = running(&shared, 1);
        st.handle(
            Event::CompareDone {
                token: 7,
                run_id: None,
                results: vec![result(0, "x")],
                save_error: Some("disk full".into()),
            },
            &mut shared,
        );
        assert!(shared.notices.iter().any(|n| n.is_error && n.text.contains("disk full")));
    }

    #[test]
    fn a_run_reports_every_model_and_is_saved() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        let (mut shared, rx) = shared();
        let mut st = CompareState::load(&shared.engine);
        st.targets = vec![ModelRef::new("custom", "a"), ModelRef::new("custom", "b")];
        st.prompt = "Say {{word}}".into();
        st.vars.insert("word".into(), "hi".into());
        st.start_run(&mut shared);
        assert!(st.running);
        assert_eq!(st.shown_prompt, "Say hi", "variables are filled before sending");
        let mut slot_events = 0;
        while st.running {
            let event = next_event(&rx);
            if matches!(event, Event::SlotDone { .. }) {
                slot_events += 1;
            }
            st.handle(event, &mut shared);
        }
        assert_eq!(slot_events, 2);
        assert!(st.slots.iter().all(|s| s.result.as_ref().unwrap().error.as_ref().unwrap().contains("Can't reach")));
        let (run, results) = shared.engine.db.load_run(st.run_id.unwrap()).unwrap();
        assert_eq!((run.prompt.as_str(), results.len()), ("Say hi", 2));
    }

    #[test]
    fn opening_a_saved_run_stops_the_current_one() {
        let (mut shared, _rx) = shared();
        let (run_id, _) = shared
            .engine
            .db
            .save_run(
                &RunRecord { kind: "compare".into(), prompt: "old".into(), ..Default::default() },
                &[result(0, "x")],
            )
            .unwrap();
        let mut st = running(&shared, 2);
        let cancel = st.run_cancel.clone().unwrap();
        st.open_run(run_id, &mut shared);
        assert!(cancel.is_cancelled());
        assert!(!st.running && st.viewing_past.is_some());
        assert_eq!((st.shown_prompt.as_str(), st.slots.len()), ("old", 1));
        st.open_run(9999, &mut shared);
        assert!(shared.notices.iter().any(|n| n.is_error), "a missing run is reported, not a crash");
    }

    #[test]
    fn run_button_explains_why_it_is_disabled() {
        let (mut shared, _rx) = shared();
        let mut st = CompareState::load(&shared.engine);
        st.targets = vec![ModelRef::new("custom", "local")];
        st.prompt.clear();
        assert_eq!(st.blocked_reason(&shared).as_deref(), Some("Write a prompt first"));
        st.prompt = "hi".into();
        assert_eq!(st.blocked_reason(&shared), None);
        st.targets.push(ModelRef::new("openai", ""));
        assert_eq!(st.blocked_reason(&shared).as_deref(), Some("Choose a model for every target"));
        st.targets[1].model = "gpt-4o".into();
        shared
            .engine
            .db
            .set_setting("budget", &Budget { monthly_cap_usd: 0.0, hard_stop: true, ..Budget::default() })
            .unwrap();
        assert!(st.blocked_reason(&shared).unwrap().contains("Monthly budget reached"));
        st.targets.pop();
        assert_eq!(st.blocked_reason(&shared), None, "local models keep working past the cap");
        st.running = true;
        assert!(st.blocked_reason(&shared).is_some());
        let _ = &mut shared;
    }

    #[test]
    fn judge_scores_land_on_the_right_answers() {
        let (mut shared, _rx) = shared();
        let mut st = running(&shared, 2);
        st.running = false;
        st.slots[0].result = Some(result(0, "a"));
        st.slots[1].result = Some(result(1, "b"));
        st.judging = true;
        st.handle(
            Event::JudgeDone { token: 7, scores: Ok(vec![(1, 9.0, "good".into()), (0, 3.0, "weak".into())]) },
            &mut shared,
        );
        assert!(!st.judging);
        assert_eq!(st.slots[0].result.as_ref().unwrap().judge_score, Some(3.0));
        assert_eq!(st.slots[1].result.as_ref().unwrap().judge_reason.as_deref(), Some("good"));
        st.handle(Event::JudgeDone { token: 7, scores: Err("judge failed".into()) }, &mut shared);
        assert_eq!(st.judge_error.as_deref(), Some("judge failed"));
    }

    #[test]
    fn exports_describe_the_run_on_screen_not_the_composer() {
        let (mut shared, _rx) = shared();
        let params = Params { temperature: 0.1, top_p: 0.9, max_tokens: 77 };
        let run = RunRecord {
            kind: "compare".into(),
            prompt: "P".into(),
            system: "System A".into(),
            params: params.clone(),
            ..Default::default()
        };
        let failed = ResultRecord { error: Some("Interrupted".into()), ..result(1, "partial") };
        let (run_id, _) = shared.engine.db.save_run(&run, &[result(0, "full"), failed]).unwrap();
        let mut st = CompareState::load(&shared.engine);
        st.system = "System B".into();
        st.params.max_tokens = 1;
        st.open_run(run_id, &mut shared);

        let json: serde_json::Value = serde_json::from_str(&crate::gui::results::export_json(&st)).unwrap();
        assert_eq!(json["system"], "System A");
        assert_eq!(json["params"]["max_tokens"], 77);
        let md = crate::gui::results::export_markdown(&st);
        assert!(md.contains("System A") && !md.contains("System B"));
        assert!(md.contains("max tokens 77"));
        assert!(md.contains("**Error:** Interrupted") && md.contains("partial"), "partial text is kept");
    }

    #[test]
    fn the_composer_is_saved_when_it_changes() {
        let (shared, _rx) = shared();
        let mut st = CompareState::load(&shared.engine);
        st.prompt = "edited without running".into();
        st.persist_if_changed(&shared.engine);
        assert_eq!(CompareState::load(&shared.engine).prompt, "edited without running");
    }

    #[test]
    fn only_answers_with_text_can_be_judged() {
        let (mut shared, _rx) = shared();
        let mut st = running(&shared, 2);
        st.running = false;
        st.slots[0].result = Some(ResultRecord { stopped: true, ..result(0, "") });
        st.slots[1].result = Some(ResultRecord { error: Some("x".into()), ..result(1, "") });
        st.start_judge(&mut shared);
        assert!(!st.judging, "nothing to score, so no judge request");
    }

    #[test]
    fn differences_view_needs_two_answers() {
        let (mut shared, _rx) = shared();
        let mut md = CommonMarkCache::default();
        let mut st = running(&shared, 1);
        st.running = false;
        st.slots[0].result = Some(result(0, "only one"));
        st.show_diff = true;
        render_all(|ui| show(ui, &mut st, &mut shared, &mut md));
        assert!(!st.show_diff);
    }

    #[test]
    fn renders_running_finished_diff_and_saved_views() {
        let (mut shared, _rx) = shared();
        let mut md = CommonMarkCache::default();
        let mut st = running(&shared, 4);
        st.slots[0].text = "partial ".repeat(200);
        st.slots[1].result = Some(ResultRecord { error: Some("Connection lost".into()), ..result(1, "half") });
        st.slots[1].text = "half".into();
        st.slot_cancels[2].cancel();
        render_all(|ui| show(ui, &mut st, &mut shared, &mut md));

        st.running = false;
        for (i, s) in st.slots.iter_mut().enumerate() {
            let mut r = result(i, "# Answer\n\nSome **markdown**.");
            r.id = Some(i as i64);
            r.rating = Some(3);
            r.judge_score = Some(7.5);
            r.judge_reason = Some("Clear and correct.".into());
            r.cpu_peak = Some(40.0);
            r.ram_peak_mb = Some(8000.0);
            r.note = Some("No price for custom/m in pricing.json".into());
            s.text = r.output.clone();
            s.result = Some(r);
        }
        render_all(|ui| show(ui, &mut st, &mut shared, &mut md));
        st.show_diff = true;
        render_all(|ui| show(ui, &mut st, &mut shared, &mut md));
        st.show_diff = false;
        st.viewing_past = Some("2026-10-16 09:00:00".into());
        st.chat_mode = true;
        st.turns = vec![Turn { user: "earlier".into(), replies: vec!["r".into(); 4] }];
        st.optimized = Some("Shorter prompt".into());
        st.prompt = "Use {{a}} and {{b}}".into();
        render_all(|ui| show(ui, &mut st, &mut shared, &mut md));
    }
}
