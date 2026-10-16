//! Chat tab: conversations with any connected model, with token-saving tools.
use std::sync::Arc;

use eframe::egui::{
    self, Align, Color32, CornerRadius, Frame, Id, Key, Layout, Margin, Modifiers, RichText, Sense, Stroke, Ui,
};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::composer::{model_picker, suggestion_card, Review};
use super::results::save_file;
use super::theme::{self, palette, Palette};
use crate::app::{Action, Event, Shared, Tab};
use crate::core_engine::chat::{context, system_prompt, title_from};
use crate::core_engine::db::{ChatMessage, ConversationRow, RunRecord};
use crate::core_engine::engine::Engine;
use crate::core_engine::llm::{Params, Prompt, Role};
use crate::core_engine::providers::ModelRef;
use crate::core_engine::token_auditor::count_tokens;

/// How many past exchanges are resent with each message.
const MEMORY_OPTIONS: [(Option<usize>, &str); 5] = [
    (Some(1), "Last exchange only"),
    (Some(3), "Last 3 exchanges"),
    (Some(6), "Last 6 exchanges"),
    (Some(12), "Last 12 exchanges"),
    (None, "Whole conversation"),
];

const READING_WIDTH: f32 = 860.0;

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    model: ModelRef,
    system: String,
    concise: bool,
    memory: Option<usize>,
    params: Params,
    optimizer: ModelRef,
}

impl Default for Saved {
    fn default() -> Self {
        Self {
            model: ModelRef::new("anthropic", "claude-opus-5-5"),
            system: String::new(),
            concise: true,
            memory: Some(6),
            params: Params::default(),
            optimizer: ModelRef::new("anthropic", "claude-opus-5-5"),
        }
    }
}

struct Streaming {
    token: u64,
    text: String,
    model: ModelRef,
    cancel: CancellationToken,
}

/// What a click on a message asked for; applied after the message list is drawn.
enum MessageAction {
    ToggleActions(usize),
    Edit(usize),
    SaveEdit,
    CancelEdit,
    Delete(usize),
    Regenerate,
}

/// Hash of everything the context token count depends on.
type ContextKey = u64;

pub struct ChatState {
    loaded: bool,
    conversations: Vec<ConversationRow>,
    current: Option<i64>,
    messages: Vec<ChatMessage>,
    model: ModelRef,
    system: String,
    concise: bool,
    memory: Option<usize>,
    params: Params,
    optimizer: ModelRef,
    input: String,
    streaming: Option<Streaming>,
    token: u64,
    optimizing: bool,
    optimized: Option<String>,
    optimize_error: Option<String>,
    optimize_token: u64,
    /// Message whose Edit/Copy/Delete actions are showing.
    actions_for: Option<usize>,
    filter: String,
    focus_input: bool,
    /// Message being edited: its index and the draft text.
    editing: Option<(usize, String)>,
    /// New title while renaming the open conversation.
    renaming: Option<String>,
    confirm_delete: bool,
    /// Context tokens for a [`ContextKey`], so long chats aren't re-tokenized every frame.
    context_cache: Option<(ContextKey, usize)>,
}

impl ChatState {
    pub fn load(engine: &Engine) -> Self {
        let saved: Saved = engine.db.get_setting("chat").unwrap_or_default();
        Self {
            loaded: false,
            conversations: vec![],
            current: None,
            messages: vec![],
            model: saved.model,
            system: saved.system,
            concise: saved.concise,
            memory: saved.memory,
            params: saved.params,
            optimizer: saved.optimizer,
            input: String::new(),
            streaming: None,
            token: 0,
            optimizing: false,
            optimized: None,
            optimize_error: None,
            optimize_token: 0,
            actions_for: None,
            filter: String::new(),
            focus_input: true,
            editing: None,
            renaming: None,
            confirm_delete: false,
            context_cache: None,
        }
    }

    fn persist(&self, engine: &Engine) {
        let saved = Saved {
            model: self.model.clone(),
            system: self.system.clone(),
            concise: self.concise,
            memory: self.memory,
            params: self.params.clone(),
            optimizer: self.optimizer.clone(),
        };
        let _ = engine.db.set_setting("chat", &saved);
    }

    fn reload_list(&mut self, shared: &mut Shared) {
        self.loaded = true;
        match shared.engine.db.list_conversations() {
            Ok(list) => self.conversations = list,
            Err(e) => shared.notice(format!("Could not load conversations: {e}"), true),
        }
    }

    fn reset_view(&mut self) {
        self.optimized = None;
        self.optimize_error = None;
        self.editing = None;
        self.actions_for = None;
        self.renaming = None;
        self.confirm_delete = false;
        self.focus_input = true;
    }

    fn open(&mut self, id: i64, shared: &mut Shared) {
        self.detach();
        match shared.engine.db.conversation_messages(id) {
            Ok(messages) => {
                self.messages = messages;
                self.current = Some(id);
                if let Some(c) = self.conversations.iter().find(|c| c.id == id) {
                    self.model = c.model.clone();
                }
                self.reset_view();
            }
            Err(e) => shared.notice(format!("Could not open the conversation: {e}"), true),
        }
    }

    fn new_chat(&mut self) {
        self.detach();
        self.current = None;
        self.messages.clear();
        self.reset_view();
    }

    fn delete_conversation(&mut self, id: i64, shared: &mut Shared) {
        if let Err(e) = shared.engine.db.delete_conversation(id) {
            shared.notice(format!("Could not delete the conversation: {e}"), true);
        }
        if self.current == Some(id) {
            self.new_chat();
        }
        self.reload_list(shared);
    }

    fn rename(&mut self, shared: &mut Shared) {
        let (Some(id), Some(title)) = (self.current, self.renaming.take()) else { return };
        let title = title.trim();
        if title.is_empty() {
            return;
        }
        match shared.engine.db.rename_conversation(id, title) {
            Ok(()) => self.reload_list(shared),
            Err(e) => shared.notice(format!("Could not rename: {e}"), true),
        }
    }

    fn export_markdown(&self) -> String {
        let title = self.title();
        let mut out = format!("# {title}\n");
        for m in &self.messages {
            let who = match (&m.role, &m.model) {
                (Role::User, _) => "You".to_string(),
                (Role::Assistant, Some(model)) => model.to_string(),
                (Role::Assistant, None) => "Assistant".to_string(),
            };
            let body = match &m.error {
                Some(e) if m.content.is_empty() => format!("_Error: {e}_"),
                Some(e) => format!("{}\n\n_Error: {e}_", m.content),
                None => m.content.clone(),
            };
            out.push_str(&format!("\n## {who}\n\n{body}\n"));
        }
        out
    }

    fn title(&self) -> String {
        self.current
            .and_then(|id| self.conversations.iter().find(|c| c.id == id))
            .map_or("New chat".to_string(), |c| c.title.clone())
    }

    fn send(&mut self, shared: &mut Shared, text: String) {
        let text = text.trim().to_string();
        if text.is_empty() || self.streaming.is_some() {
            return;
        }
        let conv = match self.current {
            Some(id) => id,
            None => match shared.engine.db.create_conversation(&title_from(&text), &self.model) {
                Ok(id) => {
                    self.current = Some(id);
                    id
                }
                Err(e) => {
                    shared.notice(format!("Could not start the conversation: {e}"), true);
                    return;
                }
            },
        };
        let mut user = ChatMessage { role: Role::User, content: text, ..Default::default() };
        match shared.engine.db.add_message(conv, &user) {
            Ok(id) => user.id = Some(id),
            Err(e) => shared.notice(format!("Could not save your message: {e}"), true),
        }
        self.messages.push(user);
        self.input.clear();
        self.optimized = None;
        self.optimize_error = None;
        self.optimizing = false;
        self.optimize_token += 1; // a suggestion still on its way is for the old draft
        self.request_reply(shared);
        self.reload_list(shared);
    }

    fn request_reply(&mut self, shared: &mut Shared) {
        let Some(conv) = self.current else { return };
        let history = context(&self.messages, self.memory);
        let Some(prompt) = history.last().filter(|m| m.role == Role::User).map(|m| m.content.clone()) else {
            return;
        };
        self.persist(&shared.engine);
        let system = system_prompt(&self.system, self.concise);
        self.token += 1;
        let token = self.token;
        let engine = Arc::clone(&shared.engine);
        let notify = shared.notify.clone();
        let model = self.model.clone();
        let params = self.params.clone();
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        tokio::spawn(async move {
            let stream = notify.clone();
            let rec = engine
                .run_one(
                    0,
                    &model,
                    Prompt { system: &system, messages: &history, params: &params },
                    &task_cancel,
                    |d| stream.send(Event::ChatDelta { token, text: d.to_string() }),
                )
                .await;
            // Also recorded as a run so chat spend counts toward the budget and analytics.
            let run = RunRecord { kind: "chat".into(), prompt, system, params, ..Default::default() };
            let _ = engine.db.save_run(&run, std::slice::from_ref(&rec));
            let mut msg = ChatMessage {
                id: None,
                role: Role::Assistant,
                content: rec.output,
                model: Some(model),
                prompt_tokens: rec.prompt_tokens,
                completion_tokens: rec.completion_tokens,
                cost_usd: rec.cost_usd,
                latency_ms: rec.latency_ms,
                error: rec.error,
                note: rec.note,
            };
            msg.id = engine.db.add_message(conv, &msg).ok();
            notify.send(Event::ChatDone { token, message: Box::new(msg) });
        });
        self.streaming = Some(Streaming { token, text: String::new(), model: self.model.clone(), cancel });
    }

    /// Stops a reply in progress, keeping what has arrived so far.
    /// The request ends cleanly: its partial text and estimated cost are saved once,
    /// by the task, and arrive through the usual ChatDone.
    fn stop(&mut self) {
        if let Some(s) = &self.streaming {
            s.cancel.cancel();
        }
    }

    /// Stops any reply and stops listening for it, before the view moves to other messages.
    /// The stopped reply is still saved to its own conversation; this view just stops waiting for it.
    fn detach(&mut self) {
        self.stop();
        self.streaming = None;
    }

    /// Replaces the last reply with a new one from the selected model.
    fn regenerate(&mut self, shared: &mut Shared) {
        if self.streaming.is_some() {
            return;
        }
        if self.messages.last().is_some_and(|m| m.role == Role::Assistant) {
            if let Some(id) = self.messages.pop().and_then(|m| m.id) {
                let _ = shared.engine.db.delete_message(id);
            }
        }
        self.request_reply(shared);
    }

    /// Saves an edited message, drops everything after it, and asks for a new reply.
    fn save_edit(&mut self, shared: &mut Shared) {
        if self.streaming.is_some() {
            return;
        }
        let Some((i, text)) = self.editing.take() else { return };
        let text = text.trim().to_string();
        if text.is_empty() || i >= self.messages.len() {
            return;
        }
        self.detach();
        if let (Some(conv), Some(id)) = (self.current, self.messages[i].id) {
            let saved = shared
                .engine
                .db
                .update_message(id, &text)
                .and_then(|_| shared.engine.db.delete_messages_after(conv, id));
            if let Err(e) = saved {
                shared.notice(format!("Could not save the edit: {e}"), true);
                return;
            }
        }
        self.messages.truncate(i + 1);
        self.messages[i].content = text;
        if self.messages[i].role == Role::User {
            self.request_reply(shared);
        }
    }

    fn delete_message(&mut self, i: usize, shared: &mut Shared) {
        if i >= self.messages.len() {
            return;
        }
        if let Some(id) = self.messages[i].id {
            if let Err(e) = shared.engine.db.delete_message(id) {
                shared.notice(format!("Could not delete the message: {e}"), true);
                return;
            }
        }
        self.messages.remove(i);
        self.editing = None;
    }

    fn start_optimize(&mut self, shared: &mut Shared) {
        if self.optimizing || self.input.trim().is_empty() {
            return;
        }
        self.persist(&shared.engine);
        self.optimizing = true;
        self.optimized = None;
        self.optimize_error = None;
        self.optimize_token += 1;
        let engine = Arc::clone(&shared.engine);
        let notify = shared.notify.clone();
        let model = self.optimizer.clone();
        let draft = self.input.clone();
        let token = self.optimize_token;
        tokio::spawn(async move {
            let result = engine.optimize_prompt(&model, &draft).await;
            notify.send(Event::ChatOptimized { token, result });
        });
    }

    pub fn handle(&mut self, event: Event, shared: &mut Shared) {
        match event {
            Event::ChatDelta { token, text } => {
                if let Some(s) = self.streaming.as_mut().filter(|s| s.token == token) {
                    s.text.push_str(&text);
                }
            }
            Event::ChatDone { token, message } => {
                if self.streaming.as_ref().is_some_and(|s| s.token == token) {
                    self.streaming = None;
                    self.messages.push(*message);
                    self.reload_list(shared);
                    self.focus_input = true;
                }
                shared.actions.push(Action::DataChanged);
            }
            Event::ChatOptimized { token, result } if token == self.optimize_token => {
                self.optimizing = false;
                match result {
                    Ok(text) => self.optimized = Some(text),
                    Err(e) => self.optimize_error = Some(e),
                }
                shared.actions.push(Action::DataChanged);
            }
            _ => {}
        }
    }

    /// Tokens the next send carries before the new message: system prompt plus remembered history.
    fn context_tokens(&mut self) -> usize {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (&self.system, self.concise, self.memory, &self.model.model).hash(&mut h);
        for m in &self.messages {
            (m.id, &m.content, m.error.is_some()).hash(&mut h);
        }
        let key = h.finish();
        if let Some((k, n)) = &self.context_cache {
            if *k == key {
                return *n;
            }
        }
        let system = system_prompt(&self.system, self.concise);
        let n = count_tokens(&system, &self.model.model).0
            + context(&self.messages, self.memory)
                .iter()
                .map(|m| count_tokens(&m.content, &self.model.model).0)
                .sum::<usize>();
        self.context_cache = Some((key, n));
        n
    }
}

pub fn show(ui: &mut Ui, state: &mut ChatState, shared: &mut Shared, md: &mut CommonMarkCache) {
    if !state.loaded {
        state.reload_list(shared);
    }
    let p = palette(ui);
    let total = ui.available_width();

    egui::Panel::left("chat_list")
        .resizable(true)
        .default_size(270.0)
        .size_range(220.0..=(total * 0.3).max(220.0))
        .frame(Frame::new().fill(p.surface).inner_margin(Margin::same(12)))
        .show(ui, |ui| theme::contained(ui, |ui| sidebar(ui, state, shared)));

    egui::Panel::bottom("chat_input")
        .frame(Frame::new().fill(p.bg).inner_margin(Margin::symmetric(24, 12)))
        .show_separator_line(false)
        .show(ui, |ui| centered(ui, |ui| input_area(ui, state, shared)));

    egui::CentralPanel::default().frame(Frame::new().fill(p.bg).inner_margin(Margin::symmetric(24, 16))).show(
        ui,
        |ui| {
            if state.messages.is_empty() && state.streaming.is_none() {
                let p = palette(ui);
                if state.current.is_some() {
                    centered(ui, |ui| header(ui, state, shared));
                }
                // About the greeting's own height (taller with the connect hint), to land at the centre.
                let info = state.model.info();
                let hint = info.needs_key && shared.key_sources.get(info.id).copied().flatten().is_none();
                let height = if hint { 150.0 } else { 70.0 };
                ui.add_space(((ui.available_height() - height) / 2.0).max(0.0));
                centered(ui, |ui| welcome(ui, state, shared, p));
                return;
            }
            centered(ui, |ui| header(ui, state, shared));
            ui.add_space(8.0);
            egui::ScrollArea::vertical().id_salt("chat_scroll").auto_shrink(false).stick_to_bottom(true).show(
                ui,
                |ui| {
                    centered(ui, |ui| conversation(ui, state, shared, md));
                },
            );
        },
    );
}

/// A column of at most READING_WIDTH, centred in the available space.
fn centered<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let width = ui.available_width().min(READING_WIDTH);
    ui.horizontal(|ui| {
        ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
        ui.vertical(|ui| {
            ui.set_width(width);
            add(ui)
        })
        .inner
    })
    .inner
}

fn sidebar(ui: &mut Ui, state: &mut ChatState, shared: &mut Shared) {
    let p = palette(ui);
    let new = egui::Button::new(RichText::new("+  New chat").strong().color(Color32::WHITE))
        .fill(p.accent)
        .min_size(egui::vec2(ui.available_width(), 34.0));
    if ui.add(new).clicked() {
        state.new_chat();
    }
    ui.add_space(8.0);
    ui.add(egui::TextEdit::singleline(&mut state.filter).hint_text("Search chats").desired_width(f32::INFINITY));
    ui.add_space(8.0);

    let needle = state.filter.to_lowercase();
    let mut open = None;
    egui::ScrollArea::vertical().id_salt("chat_list_scroll").auto_shrink(false).show(ui, |ui| {
        let visible: Vec<&ConversationRow> = state
            .conversations
            .iter()
            .filter(|c| needle.is_empty() || c.title.to_lowercase().contains(&needle))
            .collect();
        if visible.is_empty() {
            let text = if state.conversations.is_empty() { "No conversations yet." } else { "No matches." };
            ui.label(RichText::new(text).color(p.muted));
        }
        for c in visible {
            let selected = state.current == Some(c.id);
            let resp = Frame::new()
                .fill(if selected { p.control } else { Color32::TRANSPARENT })
                .corner_radius(CornerRadius::same(6))
                .inner_margin(Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(egui::Label::new(RichText::new(&c.title).strong()).truncate());
                    let when = c.updated_at.get(5..16).unwrap_or(&c.updated_at);
                    ui.add(
                        egui::Label::new(RichText::new(format!("{} · {when}", c.model.model)).small().color(p.muted))
                            .truncate(),
                    );
                })
                .response
                .interact(Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.clicked() && !selected {
                open = Some(c.id);
            }
            ui.add_space(2.0);
        }
    });
    if let Some(id) = open {
        state.open(id, shared);
    }
}

fn header(ui: &mut Ui, state: &mut ChatState, shared: &mut Shared) {
    let p = palette(ui);
    let mut delete = false;
    ui.horizontal(|ui| {
        if let Some(name) = state.renaming.as_mut() {
            let resp = ui.add(egui::TextEdit::singleline(name).font(egui::TextStyle::Heading).desired_width(360.0));
            resp.request_focus();
            let enter = ui.input(|i| i.key_pressed(Key::Enter));
            if ui.button("Save").clicked() || enter {
                state.rename(shared);
            } else if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                state.renaming = None;
            }
        } else {
            ui.add(egui::Label::new(RichText::new(state.title()).heading().strong()).truncate());
        }
        if state.current.is_some() && state.renaming.is_none() {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = if state.confirm_delete { "Click again to delete" } else { "Delete" };
                if ui.button(RichText::new(label).color(p.danger)).clicked() {
                    if state.confirm_delete {
                        delete = true;
                    } else {
                        state.confirm_delete = true;
                    }
                }
                if ui.button("Export").on_hover_text("Save this conversation as Markdown").clicked() {
                    let name =
                        format!("{}.md", state.title().replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "-"));
                    save_file(shared, &name, "Markdown", "md", state.export_markdown());
                }
                if ui.button("Rename").clicked() {
                    state.renaming = Some(state.title());
                    state.confirm_delete = false;
                }
            });
        }
    });
    if delete {
        if let Some(id) = state.current {
            state.delete_conversation(id, shared);
        }
    }

    let (tokens, cost): (usize, f64) = state
        .messages
        .iter()
        .fold((0, 0.0), |(t, c), m| (t + m.prompt_tokens + m.completion_tokens, c + m.cost_usd.unwrap_or(0.0)));
    ui.label(
        RichText::new(format!("{} · {} messages · {tokens} tokens · ${cost:.4}", state.model, state.messages.len()))
            .small()
            .color(p.muted),
    );
}

/// Chat options, shown in the Settings tab.
pub fn options(ui: &mut Ui, state: &mut ChatState, shared: &mut Shared) {
    let p = palette(ui);
    let before = (state.system.clone(), state.concise, state.memory, state.params.clone(), state.optimizer.clone());
    let before_model = state.model.clone();
    egui::Grid::new("chat_options").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
        ui.label("Model");
        ui.vertical(|ui| {
            ui.add_enabled_ui(state.streaming.is_none(), |ui| {
                ui.horizontal(|ui| {
                    ui.set_max_width(420.0);
                    model_picker(ui, "chat_model", &mut state.model, shared, 0.0);
                });
            });
            ui.label(
                RichText::new("Used for your next message. Change it any time, even mid-conversation; each reply shows which model wrote it.")
                    .small()
                    .color(p.muted),
            );
        });
        ui.end_row();

        ui.label("System prompt");
        ui.add(
            egui::TextEdit::multiline(&mut state.system)
                .hint_text("Optional instructions for every reply")
                .desired_rows(2)
                .desired_width(f32::INFINITY),
        );
        ui.end_row();

        ui.label("Concise replies");
        ui.vertical(|ui| {
            ui.checkbox(&mut state.concise, "Ask for short, direct answers");
            ui.label(
                RichText::new("Output tokens usually cost 4–5× more than input, so this saves the most.")
                    .small()
                    .color(p.muted),
            );
        });
        ui.end_row();

        ui.label("Memory");
        ui.vertical(|ui| {
            let current = MEMORY_OPTIONS.iter().find(|(m, _)| *m == state.memory).map_or("Custom", |(_, l)| l);
            egui::ComboBox::from_id_salt("chat_memory").selected_text(current).show_ui(ui, |ui| {
                for (value, label) in MEMORY_OPTIONS {
                    ui.selectable_value(&mut state.memory, value, label);
                }
            });
            ui.label(RichText::new("Older messages aren't resent, so long chats stay cheap.").small().color(p.muted));
        });
        ui.end_row();

        ui.label("Temperature");
        ui.add(egui::Slider::new(&mut state.params.temperature, 0.0..=2.0).step_by(0.05));
        ui.end_row();

        ui.label("Max reply tokens");
        ui.add(egui::DragValue::new(&mut state.params.max_tokens).range(64..=128_000).speed(16));
        ui.end_row();

        ui.label("Optimize with");
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.set_max_width(420.0);
                model_picker(ui, "chat_optimizer", &mut state.optimizer, shared, 0.0);
            });
            ui.label(
                RichText::new("Optimize suggests a clearer, shorter version of your message. You see both and choose.")
                    .small()
                    .color(p.muted),
            );
        });
        ui.end_row();
    });
    let after = (state.system.clone(), state.concise, state.memory, state.params.clone(), state.optimizer.clone());
    if before != after || before_model != state.model {
        state.persist(&shared.engine);
    }
}

fn conversation(ui: &mut Ui, state: &mut ChatState, shared: &mut Shared, md: &mut CommonMarkCache) {
    let p = palette(ui);
    let width = ui.available_width();
    let busy = state.streaming.is_some();
    let last = state.messages.len().saturating_sub(1);
    let mut action = None;

    for (i, m) in state.messages.iter().enumerate() {
        match m.role {
            Role::User => match state.editing.as_mut().filter(|(e, _)| *e == i) {
                Some((_, draft)) => {
                    ui.with_layout(Layout::top_down(Align::Max), |ui| {
                        theme::card(ui, |ui| {
                            ui.add(egui::TextEdit::multiline(draft).desired_width(f32::INFINITY).desired_rows(3));
                            ui.horizontal(|ui| {
                                let save = egui::Button::new(RichText::new("Save & resend").strong());
                                if ui
                                    .add_enabled(!busy, save)
                                    .on_disabled_hover_text("Wait for the current reply to finish")
                                    .clicked()
                                {
                                    action = Some(MessageAction::SaveEdit);
                                }
                                if ui.button("Cancel").clicked() {
                                    action = Some(MessageAction::CancelEdit);
                                }
                                ui.label(
                                    RichText::new("Later messages are replaced by a new reply.").small().color(p.muted),
                                );
                            });
                        });
                    });
                }
                None => {
                    user_bubble(ui, &m.content, width, p);
                    let open = state.actions_for == Some(i);
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        if pencil_button(ui, open).clicked() {
                            action = Some(MessageAction::ToggleActions(i));
                        }
                        if open {
                            if link(ui, "Delete", !busy) {
                                action = Some(MessageAction::Delete(i));
                            }
                            if link(ui, "Copy", true) {
                                ui.ctx().copy_text(m.content.clone());
                                action = Some(MessageAction::ToggleActions(i));
                            }
                            if link(ui, "Edit", !busy) {
                                action = Some(MessageAction::Edit(i));
                            }
                        }
                    });
                }
            },
            Role::Assistant => {
                assistant_message(ui, m.model.as_ref(), &m.content, Some(m), md, p);
                ui.horizontal(|ui| {
                    if !m.content.is_empty() && link(ui, "Copy", true) {
                        ui.ctx().copy_text(m.content.clone());
                    }
                    if link(ui, "Delete", !busy) {
                        action = Some(MessageAction::Delete(i));
                    }
                    if i == last && link(ui, &format!("Regenerate with {}", state.model.model), !busy) {
                        action = Some(MessageAction::Regenerate);
                    }
                });
            }
        }
        ui.add_space(12.0);
    }
    if let Some(s) = &state.streaming {
        assistant_message(ui, Some(&s.model), &s.text, None, md, p);
    }

    match action {
        Some(MessageAction::ToggleActions(i)) => {
            state.actions_for = if state.actions_for == Some(i) { None } else { Some(i) };
        }
        Some(MessageAction::Edit(i)) => {
            state.actions_for = None;
            state.editing = Some((i, state.messages[i].content.clone()));
        }
        Some(MessageAction::SaveEdit) => state.save_edit(shared),
        Some(MessageAction::CancelEdit) => state.editing = None,
        Some(MessageAction::Delete(i)) => {
            state.actions_for = None;
            state.delete_message(i, shared);
        }
        Some(MessageAction::Regenerate) => state.regenerate(shared),
        None => {}
    }
}

/// Pencil icon that opens a message's actions. Drawn rather than taken from a font,
/// because the default fonts don't all carry a pencil glyph.
fn pencil_button(ui: &mut Ui, open: bool) -> egui::Response {
    let p = palette(ui);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(24.0, 22.0), Sense::click());
    let resp = resp.on_hover_text("Edit, copy or delete").on_hover_cursor(egui::CursorIcon::PointingHand);
    let painter = ui.painter();
    if resp.hovered() || open {
        painter.rect_filled(rect, CornerRadius::same(5), p.control);
    }
    let color = if resp.hovered() || open { p.text } else { p.muted };
    let c = rect.center();
    let (tip, top) = (c + egui::vec2(-5.0, 5.0), c + egui::vec2(5.0, -5.0));
    // Body, then a thin nib at the writing end.
    painter.line_segment([tip + egui::vec2(2.0, -2.0), top], Stroke::new(3.0, color));
    painter.line_segment([tip, tip + egui::vec2(2.0, -2.0)], Stroke::new(1.5, color));
    if resp.has_focus() {
        painter.rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, p.accent), egui::StrokeKind::Inside);
    }
    resp
}

/// Small, quiet text button for message actions.
fn link(ui: &mut Ui, label: &str, enabled: bool) -> bool {
    let muted = palette(ui).muted;
    ui.add_enabled(enabled, egui::Button::new(RichText::new(label).small().color(muted)).frame(false)).clicked()
}

fn welcome(ui: &mut Ui, state: &ChatState, shared: &mut Shared, p: &Palette) {
    ui.vertical_centered(|ui| {
        ui.label(RichText::new("What can I help with?").size(24.0).strong());
        ui.label(RichText::new(format!("Chatting with {}", state.model)).color(p.muted));

        // Without this, the first message would just come back as an error.
        let info = state.model.info();
        let missing_key = info.needs_key && shared.key_sources.get(info.id).copied().flatten().is_none();
        let ollama_down = info.id == "ollama" && !shared.ollama.is_running;
        if missing_key || ollama_down {
            ui.add_space(6.0);
            let text = if ollama_down {
                "Ollama isn't running. Start it with `ollama serve`.".to_string()
            } else {
                format!("{} isn't connected yet.", info.short)
            };
            ui.label(RichText::new(text).small().color(p.warning));
            if missing_key && ui.link("Add an API key in Settings").clicked() {
                shared.actions.push(Action::GoTo(Tab::Settings));
            }
        }
    });
}

fn user_bubble(ui: &mut Ui, text: &str, width: f32, p: &Palette) {
    ui.with_layout(Layout::top_down(Align::Max), |ui| {
        Frame::new()
            .fill(p.accent.gamma_multiply(0.18))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_max_width(width * 0.75);
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    ui.add(egui::Label::new(RichText::new(text).color(p.text)).selectable(true));
                });
            });
    });
}

/// An assistant reply; `saved` is `None` while it is still streaming.
fn assistant_message(
    ui: &mut Ui,
    model: Option<&ModelRef>,
    text: &str,
    saved: Option<&ChatMessage>,
    md: &mut CommonMarkCache,
    p: &Palette,
) {
    ui.with_layout(Layout::top_down(Align::Min), |ui| {
        ui.horizontal(|ui| {
            if let Some(m) = model {
                theme::pill(ui, m.info().short, if m.is_local() { p.success } else { p.accent });
                ui.label(RichText::new(&m.model).small().strong());
            }
            // Stats for any reply with text (stopped and interrupted ones carry estimates). Replies
            // stopped by older versions recorded no cost, so their zeros would mislead.
            let legacy_stop = |m: &&ChatMessage| m.note.as_deref().is_some_and(|n| n.starts_with("Stopped early"));
            if let Some(m) = saved.filter(|m| !m.content.is_empty() && !legacy_stop(m)) {
                let cost = match m.cost_usd {
                    Some(0.0) => "Free".to_string(),
                    Some(c) => format!("${c:.5}"),
                    None => "price unknown".to_string(),
                };
                ui.label(
                    RichText::new(format!(
                        "{:.1} s · {} in / {} out tokens · {cost}",
                        m.latency_ms / 1000.0,
                        m.prompt_tokens,
                        m.completion_tokens
                    ))
                    .small()
                    .color(p.muted),
                );
            }
        });
        match saved.and_then(|m| m.error.as_ref()) {
            Some(err) => {
                // Keep whatever arrived before the failure.
                if !text.is_empty() {
                    CommonMarkViewer::new().show(ui, md, text);
                }
                theme::banner(ui, p.danger, |ui| {
                    ui.label(RichText::new(err).color(p.text));
                });
            }
            None if text.is_empty() && saved.is_none() => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Thinking…").color(p.muted));
                });
            }
            None => {
                CommonMarkViewer::new().show(ui, md, text);
            }
        }
        if let Some(note) = saved.and_then(|m| m.note.as_ref()) {
            ui.label(RichText::new(note).small().color(p.warning));
        }
    });
}

fn input_area(ui: &mut Ui, state: &mut ChatState, shared: &mut Shared) {
    let p = palette(ui);

    if let Some(e) = &state.optimize_error {
        ui.label(RichText::new(format!("Couldn't optimize: {e}")).small().color(p.danger));
    }
    if let Some(suggestion) = state.optimized.as_mut() {
        let model = state.model.model.clone();
        match suggestion_card(ui, &mut state.input, suggestion, &model, ("Send mine", "Send suggested")) {
            Some(Review::UseOriginal) => {
                let text = state.input.clone();
                state.send(shared, text);
            }
            Some(Review::UseSuggested) => {
                if let Some(text) = state.optimized.take() {
                    state.send(shared, text);
                }
            }
            Some(Review::Retry) => state.start_optimize(shared),
            Some(Review::Discard) => state.optimized = None,
            None => {}
        }
        ui.add_space(8.0);
        // The card holds both versions; the input box below would only repeat yours.
        if state.optimized.is_some() {
            return;
        }
    }

    let input_id = Id::new("chat_input");
    let focused = ui.memory(|m| m.has_focus(input_id));
    // Enter sends; Shift+Enter adds a line. Consumed before the editor sees it.
    let enter = focused
        && ui.input_mut(|i| {
            let hit = i.key_pressed(Key::Enter) && i.modifiers.is_none();
            if hit {
                i.consume_key(Modifiers::NONE, Key::Enter);
            }
            hit
        });

    let mut send = false;
    let mut stop = false;
    let mut optimize = false;
    Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, if focused { p.accent } else { p.border }))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let resp = ui.add(
                egui::TextEdit::multiline(&mut state.input)
                    .id(input_id)
                    .hint_text(format!("Message {}…  (Enter to send, Shift+Enter for a new line)", state.model.model))
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .frame(Frame::NONE),
            );
            if state.focus_input {
                resp.request_focus();
                state.focus_input = false;
            }
            ui.horizontal(|ui| {
                let message_tokens = count_tokens(&state.input, &state.model.model).0;
                let total = state.context_tokens() + message_tokens;
                let cost = shared.engine.pricing.read().unwrap_or_else(|e| e.into_inner()).cost(&state.model, total, 0);
                let cost = match cost {
                    Some(0.0) => "free".to_string(),
                    Some(c) => format!("~${c:.5} input"),
                    None => "price unknown".to_string(),
                };
                ui.label(
                    RichText::new(format!("{message_tokens} tokens · sends ~{total} with context · {cost}"))
                        .small()
                        .color(p.muted),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(s) = &state.streaming {
                        let stopping = s.cancel.is_cancelled();
                        let label = if stopping { "Stopping…" } else { "Stop" };
                        stop = ui
                            .add_enabled(
                                !stopping,
                                egui::Button::new(RichText::new(label).strong()).min_size(egui::vec2(72.0, 30.0)),
                            )
                            .on_hover_text("Stop the reply. The text so far is kept and its cost is estimated.")
                            .clicked();
                    } else {
                        let ready = !state.input.trim().is_empty();
                        let button = egui::Button::new(RichText::new("Send").strong().color(Color32::WHITE))
                            .fill(p.accent)
                            .min_size(egui::vec2(72.0, 30.0));
                        send = ui.add_enabled(ready, button).clicked() || (enter && ready);
                    }
                    let can_optimize = !state.input.trim().is_empty() && !state.optimizing && state.streaming.is_none();
                    let label = if state.optimizing { "Optimizing…" } else { "Optimize" };
                    optimize = ui
                        .add_enabled(can_optimize, egui::Button::new(label).min_size(egui::vec2(0.0, 30.0)))
                        .on_hover_text(format!(
                            "Suggest a clearer, shorter version of your message (with {})",
                            state.optimizer
                        ))
                        .clicked();
                    if state.optimizing {
                        ui.spinner();
                    }
                });
            });
        });

    if send {
        let text = state.input.clone();
        state.send(shared, text);
    } else if stop {
        state.stop();
    } else if optimize {
        state.start_optimize(shared);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::test_support::{next_event, render_all, shared};

    fn state(shared: &Shared) -> ChatState {
        let mut s = ChatState::load(&shared.engine);
        // Requests fail fast with "connection refused"; nothing reaches a real provider.
        s.model = ModelRef::new("custom", "mock");
        s
    }

    fn assistant(content: &str) -> ChatMessage {
        ChatMessage {
            role: Role::Assistant,
            content: content.into(),
            model: Some(ModelRef::new("custom", "mock")),
            prompt_tokens: 12,
            completion_tokens: 34,
            cost_usd: Some(0.0),
            latency_ms: 1500.0,
            ..Default::default()
        }
    }

    #[test]
    fn renders_every_state() {
        let (mut shared, _rx) = shared();
        let mut md = CommonMarkCache::default();
        let mut st = state(&shared);
        render_all(|ui| show(ui, &mut st, &mut shared, &mut md));

        st.messages = vec![
            ChatMessage { content: "word ".repeat(300), ..Default::default() },
            assistant("# Title\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n```rust\nfn main() {}\n```"),
            ChatMessage { error: Some("Connection lost mid-reply".into()), ..assistant("partial text") },
            ChatMessage { note: Some("Stopped by you; tokens and cost are estimated".into()), ..assistant("cut") },
            ChatMessage { content: "follow-up".into(), ..Default::default() },
        ];
        st.streaming = Some(Streaming {
            token: 1,
            text: "streaming…".into(),
            model: st.model.clone(),
            cancel: CancellationToken::new(),
        });
        st.actions_for = Some(0);
        render_all(|ui| show(ui, &mut st, &mut shared, &mut md));

        st.streaming = None;
        st.actions_for = None;
        st.editing = Some((0, "edited".into()));
        st.optimized = Some("A shorter prompt".into());
        st.input = "a long draft ".repeat(20);
        st.renaming = Some("New title".into());
        render_all(|ui| show(ui, &mut st, &mut shared, &mut md));
    }

    #[test]
    fn send_saves_your_message_and_the_reply() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        let (mut shared, rx) = shared();
        let mut st = state(&shared);
        st.send(&mut shared, "  Hello there  ".into());
        assert!(st.streaming.is_some());
        assert_eq!(st.messages[0].content, "Hello there");
        assert!(st.input.is_empty());

        st.handle(next_event(&rx), &mut shared);
        assert!(st.streaming.is_none());
        assert_eq!(st.messages.len(), 2);
        assert!(st.messages[1].error.as_ref().unwrap().contains("Can't reach"));
        let conv = st.current.unwrap();
        assert_eq!(shared.engine.db.conversation_messages(conv).unwrap().len(), 2);
        assert_eq!(st.conversations[0].title, "Hello there");
        assert!(shared.engine.db.history(10).unwrap().iter().any(|h| h.kind == "chat"), "spend is recorded");
    }

    #[test]
    fn blank_messages_and_sends_during_a_reply_are_ignored() {
        let (mut shared, _rx) = shared();
        let mut st = state(&shared);
        st.send(&mut shared, "   ".into());
        assert!(st.messages.is_empty() && st.current.is_none());
        st.streaming = Some(Streaming {
            token: 1,
            text: String::new(),
            model: st.model.clone(),
            cancel: CancellationToken::new(),
        });
        st.send(&mut shared, "hi".into());
        assert!(st.messages.is_empty());
    }

    #[test]
    fn replies_for_another_request_are_ignored() {
        let (mut shared, _rx) = shared();
        let mut st = state(&shared);
        st.streaming = Some(Streaming {
            token: 2,
            text: String::new(),
            model: st.model.clone(),
            cancel: CancellationToken::new(),
        });
        st.handle(Event::ChatDone { token: 1, message: Box::new(assistant("old")) }, &mut shared);
        assert!(st.messages.is_empty() && st.streaming.is_some());
        st.handle(Event::ChatDone { token: 2, message: Box::new(assistant("new")) }, &mut shared);
        assert_eq!(st.messages.len(), 1);
        assert!(st.streaming.is_none());
    }

    #[test]
    fn stop_waits_for_the_saved_reply_and_leaving_drops_it() {
        let (mut shared, _rx) = shared();
        let mut st = state(&shared);
        let cancel = CancellationToken::new();
        st.streaming =
            Some(Streaming { token: 3, text: "part".into(), model: st.model.clone(), cancel: cancel.clone() });
        st.stop();
        assert!(cancel.is_cancelled());
        assert!(st.streaming.is_some(), "the reply arrives through ChatDone");
        st.handle(Event::ChatDelta { token: 3, text: "ial".into() }, &mut shared);
        assert_eq!(st.streaming.as_ref().unwrap().text, "partial");
        st.detach();
        assert!(st.streaming.is_none());
        st.handle(Event::ChatDone { token: 3, message: Box::new(assistant("partial")) }, &mut shared);
        assert!(st.messages.is_empty(), "a reply for a chat you left isn't shown here");
    }

    #[test]
    fn editing_a_message_replaces_everything_after_it() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        let (mut shared, rx) = shared();
        let mut st = state(&shared);
        st.send(&mut shared, "first".into());
        st.handle(next_event(&rx), &mut shared);
        let conv = st.current.unwrap();

        st.editing = Some((0, "first, edited".into()));
        st.save_edit(&mut shared);
        assert_eq!(st.messages.len(), 1);
        assert_eq!(st.messages[0].content, "first, edited");
        assert!(st.streaming.is_some(), "a new reply is requested");
        let saved = shared.engine.db.conversation_messages(conv).unwrap();
        assert_eq!((saved.len(), saved[0].content.as_str()), (1, "first, edited"));

        st.handle(next_event(&rx), &mut shared);
        assert_eq!(shared.engine.db.conversation_messages(conv).unwrap().len(), 2);
    }

    #[test]
    fn delete_and_regenerate() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        let (mut shared, rx) = shared();
        let mut st = state(&shared);
        st.send(&mut shared, "hi".into());
        st.handle(next_event(&rx), &mut shared);
        let conv = st.current.unwrap();

        st.regenerate(&mut shared);
        assert_eq!(st.messages.len(), 1, "the old reply is removed");
        st.handle(next_event(&rx), &mut shared);
        assert_eq!(st.messages.len(), 2);

        st.delete_message(1, &mut shared);
        st.delete_message(0, &mut shared);
        st.delete_message(5, &mut shared); // out of range: ignored
        assert!(st.messages.is_empty());
        assert!(shared.engine.db.conversation_messages(conv).unwrap().is_empty());
    }

    #[test]
    fn rename_and_delete_a_conversation() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        let (mut shared, rx) = shared();
        let mut st = state(&shared);
        st.send(&mut shared, "hi".into());
        st.handle(next_event(&rx), &mut shared);
        let conv = st.current.unwrap();

        st.renaming = Some("   ".into());
        st.rename(&mut shared);
        assert_eq!(st.title(), "hi", "a blank title is ignored");
        st.renaming = Some("Renamed".into());
        st.rename(&mut shared);
        assert_eq!(st.title(), "Renamed");
        assert!(st.export_markdown().starts_with("# Renamed"));

        st.delete_conversation(conv, &mut shared);
        assert!(st.current.is_none() && st.messages.is_empty() && st.conversations.is_empty());
    }

    #[test]
    fn no_edits_while_a_reply_streams() {
        let (mut shared, _rx) = shared();
        let mut st = state(&shared);
        st.messages = vec![ChatMessage { content: "original".into(), ..Default::default() }];
        st.streaming = Some(Streaming {
            token: 1,
            text: String::new(),
            model: st.model.clone(),
            cancel: CancellationToken::new(),
        });
        st.editing = Some((0, "changed".into()));
        st.save_edit(&mut shared);
        assert_eq!(st.messages[0].content, "original");
        assert!(st.editing.is_some(), "the draft is kept for later");
    }

    #[test]
    fn sending_retires_a_pending_rewrite() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        let (mut shared, _rx) = shared();
        let mut st = state(&shared);
        st.optimizing = true;
        st.optimize_token = 4;
        st.send(&mut shared, "hi".into());
        assert!(!st.optimizing);
        st.handle(Event::ChatOptimized { token: 4, result: Ok("stale".into()) }, &mut shared);
        assert!(st.optimized.is_none(), "a rewrite of the old draft doesn't pop up");
    }

    #[test]
    fn export_keeps_partial_text_of_failed_replies() {
        let (shared, _rx) = shared();
        let mut st = state(&shared);
        st.messages = vec![
            ChatMessage { content: "q".into(), ..Default::default() },
            ChatMessage { error: Some("Connection lost".into()), ..assistant("half an answer") },
            ChatMessage { error: Some("No API key".into()), ..assistant("") },
        ];
        let md = st.export_markdown();
        assert!(md.contains("half an answer\n\n_Error: Connection lost_"));
        assert!(md.contains("_Error: No API key_"));
    }

    #[test]
    fn context_estimate_follows_content() {
        let (shared, _rx) = shared();
        let mut st = state(&shared);
        st.messages = vec![ChatMessage { content: "aaaa".into(), ..Default::default() }];
        let first = st.context_tokens();
        st.messages[0].content = "a much longer message with many more tokens in it".into();
        assert!(st.context_tokens() > first, "same message count, new content");
        let n = st.context_tokens();
        st.system = "x".repeat(40);
        assert!(st.context_tokens() > n);
    }

    #[test]
    fn an_emptied_conversation_still_renders_its_header() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        let (mut shared, rx) = shared();
        let mut st = state(&shared);
        st.send(&mut shared, "hi".into());
        st.handle(next_event(&rx), &mut shared);
        st.delete_message(1, &mut shared);
        st.delete_message(0, &mut shared);
        assert!(st.current.is_some() && st.messages.is_empty());
        let mut md = CommonMarkCache::default();
        render_all(|ui| show(ui, &mut st, &mut shared, &mut md));
    }

    #[test]
    fn options_are_saved_when_changed() {
        let (shared, _rx) = shared();
        let mut st = state(&shared);
        st.concise = false;
        st.memory = None;
        st.persist(&shared.engine);
        let again = ChatState::load(&shared.engine);
        assert!(!again.concise && again.memory.is_none());
        assert_eq!(again.model, ModelRef::new("custom", "mock"));
    }
}
