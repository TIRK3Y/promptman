//! Left pane of the Compare tab.
use eframe::egui::{self, Color32, Key, Modifiers, RichText, Ui};

use super::compare::{CompareState, MAX_TARGETS};
use super::theme::{self, palette};
use crate::app::Shared;
use crate::core_engine::evals::{missing_vars, template_vars};
use crate::core_engine::providers::{ModelRef, PROVIDERS};
use crate::core_engine::token_auditor::count_tokens;

pub fn show(ui: &mut Ui, state: &mut CompareState, shared: &mut Shared) {
    let p = palette(ui);
    // Consume before the editor sees it so Ctrl+Enter doesn't insert a newline.
    let shortcut = ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));

    egui::ScrollArea::vertical().id_salt("composer_scroll").show(ui, |ui| {
        theme::title(ui, "Composer", "Send one prompt to several models at once.");
        ui.add_space(8.0);

        // Targets are fixed during a conversation so each model keeps its own thread.
        let locked = state.chat_mode && !state.turns.is_empty();
        ui.horizontal(|ui| {
            theme::caption(ui, "Models");
            if locked {
                ui.label(RichText::new("locked during conversation").small().color(p.muted));
            }
        });
        theme::card(ui, |ui| {
            ui.add_enabled_ui(!locked && !state.running, |ui| {
                let mut remove = None;
                let removable = state.targets.len() > 1;
                for (i, target) in state.targets.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        model_picker(ui, ("target", i), target, shared, if removable { 40.0 } else { 0.0 });
                        if removable && ui.small_button("×").on_hover_text("Remove").clicked() {
                            remove = Some(i);
                        }
                    });
                }
                if let Some(i) = remove {
                    state.targets.remove(i);
                }
                if state.targets.len() < MAX_TARGETS && ui.button("+ Add model").clicked() {
                    let next = state.targets.last().cloned().unwrap_or_else(|| ModelRef::new("openai", "gpt-4o"));
                    state.targets.push(next);
                }
            });
        });

        ui.add_space(4.0);
        egui::CollapsingHeader::new(RichText::new("PARAMETERS").size(11.0).strong().color(p.muted))
            .id_salt("params")
            .show(ui, |ui| {
                egui::Grid::new("params_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label("Temperature");
                    ui.add(egui::Slider::new(&mut state.params.temperature, 0.0..=2.0).step_by(0.05));
                    ui.end_row();
                    ui.label("Top-p");
                    ui.add(egui::Slider::new(&mut state.params.top_p, 0.0..=1.0).step_by(0.05));
                    ui.end_row();
                    ui.label("Max tokens");
                    ui.add(egui::DragValue::new(&mut state.params.max_tokens).range(1..=128_000).speed(16));
                    ui.end_row();
                });
                ui.label(
                    RichText::new("Anthropic models don't accept temperature or top-p; they are not sent.")
                        .small()
                        .color(p.muted),
                );
            });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_enabled(!state.running, egui::Checkbox::new(&mut state.chat_mode, "Conversation mode"))
                .on_hover_text("Each run continues the conversation instead of starting fresh.");
            if !state.turns.is_empty() {
                ui.label(RichText::new(format!("{} turn(s)", state.turns.len())).small().color(p.muted));
                if ui.add_enabled(!state.running, egui::Button::new("New conversation").small()).clicked() {
                    state.new_chat();
                }
            }
        });

        ui.add_space(8.0);
        theme::caption(ui, "System prompt");
        ui.add(
            egui::TextEdit::multiline(&mut state.system)
                .hint_text("Optional instructions sent with every request")
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );

        ui.add_space(8.0);
        theme::caption(ui, if state.chat_mode && !state.turns.is_empty() { "Next message" } else { "Prompt" });
        ui.add(
            egui::TextEdit::multiline(&mut state.prompt)
                .hint_text("Write a prompt… use {{name}} for variables")
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY)
                .desired_rows(10),
        );
        let m = state.metrics();
        ui.label(
            RichText::new(format!("{} chars · {} words · {} tokens", m.char_count, m.word_count, m.token_count))
                .small()
                .color(p.muted),
        );

        ui.horizontal(|ui| {
            let ready = !state.optimizing && !state.running && !state.prompt.trim().is_empty();
            let label = if state.optimizing { "Optimizing…" } else { "Optimize prompt" };
            if ui
                .add_enabled(ready, egui::Button::new(label))
                .on_hover_text(
                    "Rewrite the prompt so it is clearer and uses fewer tokens. You review it before anything changes.",
                )
                .clicked()
            {
                state.start_optimize(shared);
            }
            if state.optimizing {
                ui.spinner();
            }
        });
        ui.label(RichText::new("Rewrite with").small().color(p.muted));
        ui.add_enabled_ui(!state.optimizing, |ui| {
            ui.horizontal(|ui| model_picker(ui, "optimizer", &mut state.optimizer_model, shared, 0.0));
        });
        if let Some(e) = &state.optimize_error {
            ui.label(RichText::new(format!("Couldn't optimize: {e}")).small().color(p.danger));
        }
        optimized_review(ui, state, shared);

        let vars = template_vars(&state.prompt);
        if !vars.is_empty() {
            ui.add_space(8.0);
            theme::caption(ui, "Variables");
            theme::card(ui, |ui| {
                egui::Grid::new("vars_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    for v in &vars {
                        ui.label(RichText::new(v).monospace());
                        let value = state.vars.entry(v.clone()).or_default();
                        ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY));
                        ui.end_row();
                    }
                });
            });
        }

        ui.add_space(12.0);
        let blocked = state.blocked_reason(shared);
        if let Some(reason) = blocked.as_ref().filter(|_| !state.running) {
            ui.label(RichText::new(reason).small().color(p.muted));
        }
        if state.running {
            let stopping = state.stopping_all();
            let label = if stopping { "Stopping…" } else { "Stop all" };
            let button =
                egui::Button::new(RichText::new(label).strong()).min_size(egui::vec2(ui.available_width(), 36.0));
            if ui
                .add_enabled(!stopping, button)
                .on_hover_text("Stop every model. Text received so far is kept.")
                .clicked()
            {
                state.stop_all();
            }
        } else {
            let button = egui::Button::new(RichText::new("Run comparison").strong().color(Color32::WHITE))
                .shortcut_text(RichText::new("Ctrl+Enter").small().color(Color32::from_white_alpha(170)))
                .fill(p.accent)
                .min_size(egui::vec2(ui.available_width(), 36.0));
            if ui.add_enabled(blocked.is_none(), button).clicked() || (shortcut && blocked.is_none()) {
                state.start_run(shared);
            }
        }
    });
}

pub enum Review {
    UseOriginal,
    UseSuggested,
    Retry,
    Discard,
}

/// Shows the user's prompt and the suggested rewrite together, both editable,
/// with their token counts, so the user picks one (or edits either) to use.
pub fn suggestion_card(
    ui: &mut Ui,
    original: &mut String,
    suggestion: &mut String,
    model: &str,
    labels: (&str, &str),
) -> Option<Review> {
    let p = palette(ui);
    let mut decision = None;
    theme::card(ui, |ui| {
        let before = count_tokens(original, model).0;
        let after = count_tokens(suggestion, model).0;
        let change = if before > 0 { (after as f64 - before as f64) / before as f64 * 100.0 } else { 0.0 };
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Choose a prompt").strong());
            if after <= before {
                theme::pill(ui, &format!("Suggested is {:.0}% shorter", -change), p.success);
            } else {
                theme::pill(ui, &format!("Suggested is {change:.0}% longer"), p.warning);
            }
        });
        let missing = missing_vars(original, suggestion);
        if !missing.is_empty() {
            ui.label(
                RichText::new(format!("The suggestion dropped these variables: {}", missing.join(", ")))
                    .small()
                    .color(p.warning),
            );
        }
        ui.add_space(4.0);

        let mut side = |ui: &mut Ui, title: &str, tokens: usize, text: &mut String, label: &str, pick: Review| {
            ui.label(RichText::new(format!("{title} · {tokens} tokens")).small().strong().color(p.muted));
            ui.add(egui::TextEdit::multiline(text).desired_width(f32::INFINITY).desired_rows(5));
            if ui.button(RichText::new(label).strong()).clicked() {
                decision = Some(pick);
            }
        };
        if ui.available_width() >= 520.0 {
            ui.columns(2, |cols| {
                side(&mut cols[0], "Your prompt", before, original, labels.0, Review::UseOriginal);
                side(&mut cols[1], "Suggested", after, suggestion, labels.1, Review::UseSuggested);
            });
        } else {
            side(ui, "Your prompt", before, original, labels.0, Review::UseOriginal);
            ui.add_space(6.0);
            side(ui, "Suggested", after, suggestion, labels.1, Review::UseSuggested);
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("Try again").clicked() {
                decision = Some(Review::Retry);
            }
            if ui.button("Close").clicked() {
                decision = Some(Review::Discard);
            }
        });
    });
    decision
}

fn optimized_review(ui: &mut Ui, state: &mut CompareState, shared: &mut Shared) {
    let model = state.targets.first().map(|t| t.model.clone()).unwrap_or_default();
    let Some(suggestion) = state.optimized.as_mut() else { return };
    ui.add_space(8.0);
    match suggestion_card(ui, &mut state.prompt, suggestion, &model, ("Keep mine", "Use suggested")) {
        Some(Review::UseSuggested) => {
            if let Some(text) = state.optimized.take() {
                state.prompt = text;
            }
        }
        Some(Review::UseOriginal) | Some(Review::Discard) => state.optimized = None,
        Some(Review::Retry) => state.start_optimize(shared),
        None => {}
    }
}

/// Provider dropdown plus a model dropdown that also accepts any model id.
/// The model dropdown fills the row, leaving `reserve` pixels for widgets after it.
pub fn model_picker(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug + Copy,
    m: &mut ModelRef,
    shared: &Shared,
    reserve: f32,
) {
    let provider_width = ((ui.available_width() - reserve) * 0.4).clamp(64.0, 100.0);
    sized(ui, provider_width, |ui| {
        egui::ComboBox::from_id_salt((id, "provider"))
            .selected_text(m.info().short)
            .width(provider_width)
            .truncate()
            .show_ui(ui, |ui| {
                for prov in PROVIDERS {
                    if ui.selectable_label(m.provider == prov.id, prov.name).clicked() && m.provider != prov.id {
                        m.provider = prov.id.to_string();
                        m.model = suggestions(prov.id, shared).into_iter().next().unwrap_or_default();
                    }
                }
            });
    });

    let options = suggestions(&m.provider, shared);
    let muted = palette(ui).muted;
    // Measured after the provider dropdown is placed, so the row never overflows.
    let width = (ui.available_width() - reserve - ui.spacing().item_spacing.x).max(48.0);
    sized(ui, width, |ui| model_combo(ui, id, m, &options, muted, width));
}

fn model_combo(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    m: &mut ModelRef,
    options: &[String],
    muted: Color32,
    width: f32,
) {
    let shown = if m.model.is_empty() { "Choose a model".to_string() } else { m.model.clone() };
    egui::ComboBox::from_id_salt((id, "model")).selected_text(shown).width(width).truncate().show_ui(ui, |ui| {
        for o in options {
            ui.selectable_value(&mut m.model, o.clone(), o);
        }
        if options.is_empty() && m.provider == "ollama" {
            ui.label(RichText::new("No installed models found").small().color(muted));
        }
        ui.separator();
        ui.label(RichText::new("Other model id").small().color(muted));
        ui.text_edit_singleline(&mut m.model);
    });
}

/// Lays out `add` in a box exactly `width` wide. Truncating combo boxes size their
/// text to the available width, so this is what keeps them from overflowing.
fn sized<R>(ui: &mut Ui, width: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let size = egui::vec2(width, ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_max_width(width);
        add(ui)
    })
    .inner
}

fn suggestions(provider: &str, shared: &Shared) -> Vec<String> {
    if provider == "ollama" {
        shared.ollama.available_models.iter().map(|m| m.name.clone()).collect()
    } else {
        shared.engine.pricing.read().unwrap_or_else(|e| e.into_inner()).models_for(provider)
    }
}
