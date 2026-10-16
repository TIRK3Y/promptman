//! Library tab: saved, tagged, versioned prompts.
use eframe::egui::{self, Frame, Margin, RichText, Ui};

use super::compare::CompareState;
use super::theme::{self, palette};
use crate::app::{Action, Shared, Tab};
use crate::core_engine::db::PromptRecord;

#[derive(Default)]
pub struct LibraryState {
    loaded: bool,
    prompts: Vec<PromptRecord>,
    filter: String,
    selected: Option<String>,
    versions: Vec<PromptRecord>,
    version_idx: usize,
    save_name: String,
    save_tags: String,
    confirm_delete: bool,
}

impl LibraryState {
    fn reload(&mut self, shared: &mut Shared) {
        self.loaded = true;
        match shared.engine.db.list_prompts() {
            Ok(p) => self.prompts = p,
            Err(e) => shared.notice(format!("Could not load the library: {e}"), true),
        }
        if let Some(name) = self.selected.clone() {
            self.select(&name, shared);
        }
    }

    fn select(&mut self, name: &str, shared: &Shared) {
        self.versions = shared.engine.db.prompt_versions(name).unwrap_or_default();
        self.selected = (!self.versions.is_empty()).then(|| name.to_string());
        self.version_idx = 0;
        self.confirm_delete = false;
    }
}

pub fn show(ui: &mut Ui, state: &mut LibraryState, compare: &mut CompareState, shared: &mut Shared) {
    if !state.loaded {
        state.reload(shared);
    }
    let p = palette(ui);
    let body = Frame::new().fill(p.bg).inner_margin(Margin::same(16));

    let total = ui.available_width();
    egui::Panel::left("library_list")
        .resizable(true)
        .default_size(320.0)
        .size_range(260.0..=(total * 0.45).max(260.0))
        .frame(body)
        .show(ui, |ui| {
            theme::contained(ui, |ui| {
                theme::title(ui, "Library", "Saved prompts with version history.");
                ui.add_space(8.0);
                ui.add(
                    egui::TextEdit::singleline(&mut state.filter)
                        .hint_text("Search by name or tag")
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(8.0);
                let needle = state.filter.to_lowercase();
                let mut clicked = None;
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    let matches: Vec<&PromptRecord> = state
                        .prompts
                        .iter()
                        .filter(|pr| {
                            needle.is_empty()
                                || pr.name.to_lowercase().contains(&needle)
                                || pr.tags.to_lowercase().contains(&needle)
                        })
                        .collect();
                    if matches.is_empty() {
                        ui.label(
                            RichText::new(if state.prompts.is_empty() { "Nothing saved yet." } else { "No matches." })
                                .color(p.muted),
                        );
                    }
                    for pr in matches {
                        let selected = state.selected.as_deref() == Some(pr.name.as_str());
                        let resp = ui.selectable_label(selected, RichText::new(&pr.name).strong());
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(format!("v{} · {}", pr.version, pr.created_at)).small().color(p.muted),
                            );
                            for tag in pr.tags.split(',').map(str::trim).filter(|t| !t.is_empty()) {
                                theme::pill(ui, tag, p.accent);
                            }
                        });
                        ui.add_space(4.0);
                        if resp.clicked() {
                            clicked = Some(pr.name.clone());
                        }
                    }
                });
                if let Some(name) = clicked {
                    state.select(&name, shared);
                }
            })
        });

    egui::CentralPanel::default().frame(body).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            theme::caption(ui, "Save the Composer prompt");
            theme::card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut state.save_name).hint_text("Name").desired_width(220.0));
                    ui.add(
                        egui::TextEdit::singleline(&mut state.save_tags)
                            .hint_text("Tags, comma separated")
                            .desired_width(220.0),
                    );
                    let ready = !state.save_name.trim().is_empty() && !compare.prompt.trim().is_empty();
                    if ui.add_enabled(ready, egui::Button::new("Save")).clicked() {
                        let name = state.save_name.trim().to_string();
                        match shared.engine.db.save_prompt(
                            &name,
                            state.save_tags.trim(),
                            &compare.system,
                            &compare.prompt,
                        ) {
                            Ok(v) => {
                                shared.notice(format!("Saved \"{name}\" as version {v}."), false);
                                state.selected = Some(name);
                                state.reload(shared);
                            }
                            Err(e) => shared.notice(format!("Could not save: {e}"), true),
                        }
                    }
                });
                ui.label(
                    RichText::new("Saving under an existing name adds a new version; older versions are kept.")
                        .small()
                        .color(p.muted),
                );
            });
            ui.add_space(16.0);

            let Some(name) = state.selected.clone() else {
                ui.label(RichText::new("Select a prompt to see it here.").color(p.muted));
                return;
            };
            let Some(current) = state.versions.get(state.version_idx).cloned() else { return };

            ui.horizontal(|ui| {
                ui.label(RichText::new(&name).heading().strong());
                egui::ComboBox::from_id_salt("prompt_version").selected_text(format!("v{}", current.version)).show_ui(
                    ui,
                    |ui| {
                        for (i, v) in state.versions.iter().enumerate() {
                            ui.selectable_value(
                                &mut state.version_idx,
                                i,
                                format!("v{} · {}", v.version, v.created_at),
                            );
                        }
                    },
                );
            });
            if !current.tags.is_empty() {
                ui.label(RichText::new(format!("Tags: {}", current.tags)).color(p.muted));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Load into Composer").strong()).clicked() {
                    compare.system = current.system.clone();
                    compare.prompt = current.body.clone();
                    compare.persist_if_changed(&shared.engine);
                    compare.new_chat();
                    shared.actions.push(Action::GoTo(Tab::Compare));
                }
                let delete_label = if state.confirm_delete { "Click again to delete all versions" } else { "Delete" };
                if ui.button(RichText::new(delete_label).color(p.danger)).clicked() {
                    if state.confirm_delete {
                        match shared.engine.db.delete_prompt(&name) {
                            Ok(()) => {
                                state.selected = None;
                                state.reload(shared);
                            }
                            Err(e) => shared.notice(format!("Could not delete: {e}"), true),
                        }
                    } else {
                        state.confirm_delete = true;
                    }
                }
            });
            ui.add_space(8.0);
            if !current.system.trim().is_empty() {
                theme::caption(ui, "System prompt");
                theme::card(ui, |ui| ui.label(RichText::new(&current.system).monospace()));
                ui.add_space(8.0);
            }
            theme::caption(ui, "Prompt");
            theme::card(ui, |ui| ui.label(RichText::new(&current.body).monospace()));
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::test_support::{render_all, shared};

    #[test]
    fn renders_and_selects_versions() {
        let (mut shared, _rx) = shared();
        let mut compare = CompareState::load(&shared.engine);
        let mut st = LibraryState::default();
        render_all(|ui| show(ui, &mut st, &mut compare, &mut shared));
        shared.engine.db.save_prompt("Summarizer", "a, b", "Be brief.", "v1").unwrap();
        shared.engine.db.save_prompt("Summarizer", "a, b", "Be brief.", "v2").unwrap();
        shared.engine.db.save_prompt("Translator", "", "", "Translate {{text}}").unwrap();
        st.reload(&mut shared);
        st.select("Summarizer", &shared);
        assert_eq!(st.versions.len(), 2);
        assert_eq!(st.versions[0].body, "v2", "newest version first");
        st.filter = "transl".into();
        st.confirm_delete = true;
        render_all(|ui| show(ui, &mut st, &mut compare, &mut shared));
        st.select("Missing", &shared);
        assert!(st.selected.is_none());
    }
}
