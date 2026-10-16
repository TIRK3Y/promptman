//! Centre pane of the Compare tab: answers, diff, rating, judge, export.
use eframe::egui::{self, text::LayoutJob, FontId, RichText, Stroke, TextFormat, Ui};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use similar::{ChangeTag, TextDiff};

use super::compare::{CompareState, Slot};
use super::composer::model_picker;
use super::theme::{self, palette, Palette};
use crate::app::Shared;

pub fn show(ui: &mut Ui, state: &mut CompareState, shared: &mut Shared, md: &mut CommonMarkCache) {
    let p = palette(ui);

    let (title, subtitle) = match &state.viewing_past {
        Some(when) => (format!("Saved run · {when}"), state.shown_prompt.clone()),
        None if !state.slots.is_empty() => ("Results".to_string(), state.shown_prompt.clone()),
        None => ("Results".to_string(), "Answers appear here as they stream in.".to_string()),
    };
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_max_width((ui.available_width() - 120.0).max(0.0));
            ui.label(RichText::new(title).heading().strong());
            ui.add(egui::Label::new(RichText::new(subtitle.replace('\n', " ")).color(p.muted)).truncate());
        });
        if state.viewing_past.is_some() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if ui.button("Close").clicked() {
                    state.viewing_past = None;
                    state.slots.clear();
                    state.run_id = None;
                }
            });
        }
    });

    if state.slots.is_empty() {
        empty_state(ui, p, "No results yet", "Write a prompt and press Run comparison (Ctrl+Enter).");
        return;
    }

    ui.add_space(4.0);
    toolbar(ui, state, shared);
    if let Some(e) = &state.judge_error {
        ui.label(RichText::new(format!("Judge failed: {e}")).small().color(p.danger));
    }
    ui.add_space(4.0);

    if state.show_diff {
        diff_view(ui, state, p);
        return;
    }

    let n = state.slots.len();
    let cols = n.min(2);
    let rows = n.div_ceil(cols);
    let gap = 8.0;
    let row_height = ((ui.available_height() - gap * (rows - 1) as f32) / rows as f32).max(120.0);
    let mut rating = None;
    let mut stop = None;
    for row in 0..rows {
        ui.columns(cols, |columns| {
            for (c, col) in columns.iter_mut().enumerate() {
                let i = row * cols + c;
                if i < n {
                    // Columns are justified; cards lay out ragged-left so short lines aren't stretched.
                    let rated = col.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        card(ui, i, state, row_height, md, p, &mut stop)
                    });
                    if let Some(r) = rated.inner {
                        rating = Some((i, r));
                    }
                }
            }
        });
        if row + 1 < rows {
            ui.add_space(gap);
        }
    }
    if let Some((slot, r)) = rating {
        state.set_rating(slot, r, shared);
    }
    if let Some(slot) = stop {
        state.stop_slot(slot);
    }
}

fn empty_state(ui: &mut Ui, p: &Palette, heading: &str, body: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.35);
        ui.label(RichText::new(heading).strong());
        ui.label(RichText::new(body).color(p.muted));
    });
}

fn toolbar(ui: &mut Ui, state: &mut CompareState, shared: &mut Shared) {
    let p = palette(ui);
    if state.slots.len() < 2 {
        state.show_diff = false;
    }
    let finished = !state.running && state.slots.iter().any(|s| s.result.is_some());
    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(&mut state.show_diff, false, "Answers");
        ui.add_enabled_ui(state.slots.len() >= 2, |ui| {
            ui.selectable_value(&mut state.show_diff, true, "Differences");
        });
        ui.checkbox(&mut state.markdown, "Render markdown");
        ui.add_enabled_ui(finished, |ui| {
            ui.menu_button("Export", |ui| {
                if ui.button("Markdown…").clicked() {
                    save_file(shared, "promptman-run.md", "Markdown", "md", export_markdown(state));
                    ui.close();
                }
                if ui.button("JSON…").clicked() {
                    save_file(shared, "promptman-run.json", "JSON", "json", export_json(state));
                    ui.close();
                }
            });
        });
    });
    // Own row: the model picker is too wide to share a line with the view controls. On narrow
    // panes the Score button moves to a second row so the picker keeps a usable width.
    let narrow = ui.available_width() < 560.0;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Judge").small().color(p.muted));
        ui.add_enabled_ui(!state.judging, |ui| {
            model_picker(ui, "judge", &mut state.judge_model, shared, if narrow { 0.0 } else { 140.0 });
        });
        if !narrow {
            judge_button(ui, state, shared, finished);
        }
    });
    if narrow {
        ui.horizontal(|ui| judge_button(ui, state, shared, finished));
    }
}

fn judge_button(ui: &mut Ui, state: &mut CompareState, shared: &mut Shared, finished: bool) {
    let can_judge = finished
        && !state.judging
        && state
            .slots
            .iter()
            .any(|s| s.result.as_ref().is_some_and(|r| r.error.is_none() && !r.output.trim().is_empty()));
    if ui
        .add_enabled(can_judge, egui::Button::new(if state.judging { "Judging…" } else { "Score answers" }))
        .on_hover_text("Ask the judge model to score each answer from 0 to 10")
        .clicked()
    {
        state.start_judge(shared);
    }
    if state.judging {
        ui.spinner();
    }
}

/// Draws one answer card; returns a new rating if the user clicked a star.
fn card(
    ui: &mut Ui,
    i: usize,
    state: &CompareState,
    height: f32,
    md: &mut CommonMarkCache,
    p: &Palette,
    stop: &mut Option<usize>,
) -> Option<Option<u8>> {
    let slot: &Slot = &state.slots[i];
    let color = if slot.target.is_local() { p.success } else { p.accent };
    let mut new_rating = None;

    theme::card(ui, |ui| {
        // Card adds 14px margin + 1px stroke on each side.
        let inner = (height - 30.0).max(60.0);
        ui.set_min_height(inner);
        ui.set_max_height(inner);
        // Buttons claim their space first (right to left) and the model name truncates into the rest.
        // Cards too narrow for both get two rows: who answered, then the actions.
        let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
        let identity = |ui: &mut Ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                theme::pill(ui, slot.target.info().short, color);
                ui.add(egui::Label::new(RichText::new(&slot.target.model).strong()).truncate());
            });
        };
        let narrow = row.x < 240.0;
        if narrow {
            ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), identity);
        }
        ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !slot.text.is_empty() && ui.small_button("Copy").clicked() {
                ui.ctx().copy_text(slot.text.clone());
            }
            if state.running && slot.result.is_none() {
                let stopping = state.slot_stopping(i);
                let label = if stopping { "Stopping…" } else { "Stop" };
                if ui.add_enabled(!stopping, egui::Button::new(label).small()).clicked() {
                    *stop = Some(i);
                }
            }
            if !narrow {
                identity(ui);
            }
        });

        // Answers that failed before any text arrived show only their error, below.
        // A valid empty answer still gets its stats; one that failed or was stopped before any text doesn't.
        if let Some(r) = slot.result.as_ref().filter(|r| !r.output.is_empty() || (r.error.is_none() && !r.stopped)) {
            let tokens = format!(
                "{}{} tokens",
                if r.tokens_estimated { "~" } else { "" },
                r.prompt_tokens + r.completion_tokens
            );
            let mut parts = vec![format!("{:.0} ms", r.latency_ms)];
            if let Some(t) = r.ttft_ms {
                parts.push(format!("first token {t:.0} ms"));
            }
            parts.push(tokens);
            parts.push(format!("{:.1} tok/s", r.tokens_per_sec));
            parts.push(match r.cost_usd {
                Some(0.0) => "Free".to_string(),
                Some(c) => format!("${c:.6}"),
                None => "price unknown".to_string(),
            });
            ui.label(RichText::new(parts.join("  ·  ")).small().monospace().color(p.muted));

            if let (Some(cpu), Some(ram)) = (r.cpu_peak, r.ram_peak_mb) {
                let mut hw = format!("Peak CPU {cpu:.0}%  ·  RAM {:.1} GB", ram / 1024.0);
                if let Some(g) = r.gpu_peak {
                    hw.push_str(&format!("  ·  GPU {g:.0}%"));
                }
                ui.label(RichText::new(hw).small().monospace().color(p.muted));
            }
            if let Some(note) = &r.note {
                ui.label(RichText::new(note).small().color(p.warning));
            }

            ui.horizontal(|ui| {
                if r.id.is_some() && r.error.is_none() {
                    let current = r.rating.unwrap_or(0);
                    for star in 1..=5u8 {
                        let c = if star <= current { p.warning } else { p.muted.gamma_multiply(0.5) };
                        let resp = ui
                            .add(egui::Button::new(RichText::new("★").size(16.0).color(c)).frame(false))
                            .on_hover_text(format!("Rate {star}/5"));
                        if resp.clicked() {
                            new_rating = Some(if current == star { None } else { Some(star) });
                        }
                    }
                }
                if let Some(score) = r.judge_score {
                    ui.add_space(8.0);
                    theme::pill(ui, &format!("Judge {score:.1}/10"), p.accent);
                }
            });
            if let Some(reason) = r.judge_reason.as_ref().filter(|s| !s.is_empty()) {
                ui.label(RichText::new(reason).small().italics().color(p.muted));
            }
        }
        ui.separator();

        egui::ScrollArea::vertical()
            .id_salt(("card_scroll", i))
            .auto_shrink(false)
            .stick_to_bottom(state.running)
            .show(ui, |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    let show_turns = state.chat_mode && state.viewing_past.is_none();
                    if show_turns {
                        for turn in &state.turns {
                            ui.label(RichText::new("You").small().strong().color(p.muted));
                            ui.label(RichText::new(&turn.user).color(p.muted));
                            ui.add_space(4.0);
                            body_text(ui, md, turn.replies.get(i).map_or("", |s| s), state.markdown);
                            ui.separator();
                        }
                    }
                    if show_turns && state.slots_in_turns {
                        return;
                    }
                    if show_turns && !state.turns.is_empty() {
                        ui.label(RichText::new("You").small().strong().color(p.muted));
                        ui.label(RichText::new(&state.shown_prompt).color(p.muted));
                        ui.add_space(4.0);
                    }
                    match slot.result.as_ref().and_then(|r| r.error.as_ref()) {
                        Some(err) => {
                            ui.label(RichText::new(err).color(p.danger));
                            // Keep whatever arrived before the failure.
                            if !slot.text.is_empty() {
                                ui.add_space(4.0);
                                body_text(ui, md, &slot.text, state.markdown);
                            }
                        }
                        None if slot.text.is_empty() && state.running => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Waiting for the first token…").color(p.muted));
                            });
                        }
                        None => body_text(ui, md, &slot.text, state.markdown && !state.running),
                    }
                });
            });
    });
    new_rating
}

fn body_text(ui: &mut Ui, md: &mut CommonMarkCache, text: &str, markdown: bool) {
    if markdown {
        CommonMarkViewer::new().show(ui, md, text);
    } else {
        ui.label(RichText::new(text).monospace());
    }
}

fn diff_view(ui: &mut Ui, state: &mut CompareState, p: &Palette) {
    let labels: Vec<String> = state.slots.iter().map(|s| s.target.to_string()).collect();
    ui.horizontal(|ui| {
        for (which, id) in [(0, "diff_a"), (1, "diff_b")] {
            let idx = if which == 0 { &mut state.diff_pair.0 } else { &mut state.diff_pair.1 };
            *idx = (*idx).min(labels.len() - 1);
            egui::ComboBox::from_id_salt(id).selected_text(&labels[*idx]).width(220.0).show_ui(ui, |ui| {
                for (i, l) in labels.iter().enumerate() {
                    ui.selectable_value(idx, i, l);
                }
            });
            if which == 0 {
                ui.label("vs");
            }
        }
        ui.add_space(12.0);
        theme::pill(ui, "Only in first", p.danger);
        theme::pill(ui, "Only in second", p.success);
    });
    ui.add_space(8.0);

    let a = &state.slots[state.diff_pair.0].text;
    let b = &state.slots[state.diff_pair.1].text;
    let font = FontId::monospace(13.0);
    let mut job = LayoutJob::default();
    job.wrap.max_width = (ui.available_width() - 32.0).max(100.0);
    for change in TextDiff::from_words(a.as_str(), b.as_str()).iter_all_changes() {
        let format = match change.tag() {
            ChangeTag::Equal => TextFormat { font_id: font.clone(), color: p.text, ..Default::default() },
            ChangeTag::Delete => TextFormat {
                font_id: font.clone(),
                color: p.danger,
                background: p.danger.gamma_multiply(0.15),
                strikethrough: Stroke::new(1.0, p.danger),
                ..Default::default()
            },
            ChangeTag::Insert => TextFormat {
                font_id: font.clone(),
                color: p.success,
                background: p.success.gamma_multiply(0.15),
                ..Default::default()
            },
        };
        job.append(change.value(), 0.0, format);
    }
    theme::card(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("diff_scroll").auto_shrink(false).show(ui, |ui| {
            ui.label(job);
        });
    });
}

pub(crate) fn export_markdown(state: &CompareState) -> String {
    let mut s = format!("# PromptMan run\n\n## Prompt\n\n{}\n", state.shown_prompt);
    if !state.shown_system.trim().is_empty() {
        s.push_str(&format!("\n## System prompt\n\n{}\n", state.shown_system));
    }
    let p = &state.shown_params;
    s.push_str(&format!("\n_Temperature {} · top-p {} · max tokens {}_\n", p.temperature, p.top_p, p.max_tokens));
    for slot in &state.slots {
        s.push_str(&format!("\n## {}\n\n", slot.target));
        if let Some(r) = &slot.result {
            s.push_str(&format!(
                "_{:.0} ms · {} prompt + {} completion tokens · {}_\n\n",
                r.latency_ms,
                r.prompt_tokens,
                r.completion_tokens,
                r.cost_usd.map_or("price unknown".into(), |c| format!("${c:.6}")),
            ));
            if let Some(e) = &r.error {
                s.push_str(&format!("**Error:** {e}\n\n"));
            }
        }
        s.push_str(&slot.text);
        s.push('\n');
    }
    s
}

pub(crate) fn export_json(state: &CompareState) -> String {
    let results: Vec<_> = state.slots.iter().filter_map(|s| s.result.as_ref()).collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "prompt": state.shown_prompt,
        "system": state.shown_system,
        "params": state.shown_params,
        "results": results,
    }))
    .unwrap_or_default()
}

pub fn save_file(shared: &mut Shared, name: &str, kind: &str, ext: &str, contents: String) {
    let Some(path) = rfd::FileDialog::new().set_file_name(name).add_filter(kind, &[ext]).save_file() else {
        return;
    };
    match std::fs::write(&path, contents) {
        Ok(()) => shared.notice(format!("Saved {}", path.display()), false),
        Err(e) => shared.notice(format!("Could not save {}: {e}", path.display()), true),
    }
}
