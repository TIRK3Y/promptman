//! Batch tab: run the composer template over CSV rows and check each answer.
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, Frame, Margin, ProgressBar, RichText, Ui};
use tokio_util::sync::CancellationToken;

use super::compare::CompareState;
use super::results::save_file;
use super::theme::{self, palette};
use crate::app::{Action, Event, Shared};
use crate::core_engine::db::{ResultRecord, RunRecord};
use crate::core_engine::engine::RunHooks;
use crate::core_engine::evals::{fill_template, load_csv, run_checks, template_vars, Check, CsvTable};
use crate::core_engine::llm::Message;
use crate::core_engine::providers::ModelRef;

pub struct Cell {
    pub slot: usize,
    pub result: ResultRecord,
    pub passed: bool,
    pub failures: Vec<String>,
}

const CHECK_KINDS: [&str; 4] = ["Contains", "Does not contain", "Matches regex", "Is valid JSON"];

#[derive(Default)]
pub struct BatchState {
    csv_path: Option<PathBuf>,
    table: Option<CsvTable>,
    checks: Vec<Check>,
    new_kind: usize,
    new_value: String,
    /// `cells[row][slot]`
    cells: Vec<Vec<Option<Cell>>>,
    targets: Vec<ModelRef>,
    running: bool,
    token: u64,
    done: usize,
    total: usize,
    /// Stops the run, including the row in progress.
    cancel: CancellationToken,
}

impl BatchState {
    fn template(&self, compare: &CompareState) -> String {
        let has_prompt_col = self.table.as_ref().is_some_and(|t| t.headers.iter().any(|h| h == "prompt"));
        if compare.prompt.trim().is_empty() && has_prompt_col {
            "{{prompt}}".to_string()
        } else {
            compare.prompt.clone()
        }
    }

    fn start(&mut self, compare: &CompareState, shared: &mut Shared) {
        let Some(table) = &self.table else { return };
        let rows = table.rows.clone();
        let template = self.template(compare);
        let targets = compare.targets.clone();
        let system = compare.system.clone();
        let params = compare.params.clone();
        let checks = self.checks.clone();

        self.token += 1;
        self.running = true;
        self.done = 0;
        self.total = rows.len() * targets.len();
        self.cells = rows.iter().map(|_| targets.iter().map(|_| None).collect()).collect();
        self.targets = targets.clone();
        self.cancel = CancellationToken::new();

        let cancel = self.cancel.clone();
        let engine = Arc::clone(&shared.engine);
        let notify = shared.notify.clone();
        let token = self.token;
        tokio::spawn(async move {
            for (row, vars) in rows.iter().enumerate() {
                if cancel.is_cancelled() {
                    break;
                }
                let prompt = fill_template(&template, vars);
                let convs = vec![vec![Message::user(prompt.clone())]; targets.len()];
                let cancels = targets.iter().map(|_| cancel.child_token()).collect();
                let results = engine.run_all(&targets, &system, convs, &params, cancels, RunHooks::none()).await;
                let run = RunRecord {
                    kind: "batch".into(),
                    prompt,
                    system: system.clone(),
                    params: params.clone(),
                    ..Default::default()
                };
                let _ = engine.db.save_run(&run, &results);
                for result in results {
                    let (passed, failures) = match &result.error {
                        Some(e) => (false, vec![format!("error: {e}")]),
                        None if result.stopped => (false, vec!["stopped before finishing".to_string()]),
                        None => run_checks(&checks, vars.get("expected").map(String::as_str), &result.output),
                    };
                    let cell = Cell { slot: result.slot, result, passed, failures };
                    notify.send(Event::BatchCell { token, row, cell: Box::new(cell) });
                }
            }
            notify.send(Event::BatchDone { token });
        });
    }

    pub fn handle(&mut self, event: Event, shared: &mut Shared) {
        match event {
            Event::BatchCell { token, row, cell } if token == self.token => {
                if let Some(slot) = self.cells.get_mut(row).and_then(|r| r.get_mut(cell.slot)) {
                    *slot = Some(*cell);
                    self.done += 1;
                }
            }
            Event::BatchDone { token } if token == self.token => {
                self.running = false;
                shared.actions.push(Action::DataChanged);
            }
            _ => {}
        }
    }

    fn export_csv(&self) -> Result<String, String> {
        let table = self.table.as_ref().ok_or("no data")?;
        let mut w = csv::Writer::from_writer(vec![]);
        let mut header: Vec<String> = vec!["row".into()];
        header.extend(table.headers.iter().cloned());
        header.extend(["model", "passed", "failures", "latency_ms", "cost_usd", "output"].map(String::from));
        w.write_record(&header).map_err(|e| e.to_string())?;
        for (row, cells) in self.cells.iter().enumerate() {
            for cell in cells.iter().flatten() {
                let mut rec = vec![row.to_string()];
                rec.extend(table.headers.iter().map(|h| table.rows[row].get(h).cloned().unwrap_or_default()));
                rec.push(cell.result.model_ref());
                rec.push(cell.passed.to_string());
                rec.push(cell.failures.join("; "));
                rec.push(format!("{:.0}", cell.result.latency_ms));
                rec.push(cell.result.cost_usd.map(|c| c.to_string()).unwrap_or_default());
                rec.push(cell.result.output.clone());
                w.write_record(&rec).map_err(|e| e.to_string())?;
            }
        }
        String::from_utf8(w.into_inner().map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }
}

pub fn show(ui: &mut Ui, state: &mut BatchState, compare: &CompareState, shared: &mut Shared) {
    let p = palette(ui);
    egui::CentralPanel::default().frame(Frame::new().fill(p.bg).inner_margin(Margin::same(16))).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            theme::title(ui, "Batch & checks", "Run the Composer prompt once per CSV row, then check every answer.");
            ui.add_space(12.0);
            setup(ui, state, compare, shared);
            ui.add_space(12.0);
            checks(ui, state);
            ui.add_space(12.0);
            run_controls(ui, state, compare, shared);
            if !state.cells.is_empty() {
                ui.add_space(12.0);
                summary(ui, state);
                ui.add_space(12.0);
                grid(ui, state);
            }
        });
    });
}

fn setup(ui: &mut Ui, state: &mut BatchState, compare: &CompareState, shared: &mut Shared) {
    let p = palette(ui);
    theme::caption(ui, "Inputs");
    theme::card(ui, |ui| {
        let template = state.template(compare);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Template").strong());
            ui.label(RichText::new("from the Composer").small().color(p.muted));
            if ui.small_button("Edit").clicked() {
                shared.actions.push(Action::GoTo(crate::app::Tab::Compare));
            }
        });
        ui.add(egui::Label::new(RichText::new(template.replace('\n', " ")).monospace().color(p.muted)).truncate());
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Models").strong());
            for t in &compare.targets {
                theme::pill(ui, &t.to_string(), if t.is_local() { p.success } else { p.accent });
            }
        });
        ui.separator();

        ui.horizontal(|ui| {
            // Replacing the CSV mid-run would leave the run spending on rows nobody sees.
            if ui.add_enabled(!state.running, egui::Button::new("Choose CSV…")).clicked() {
                if let Some(path) = rfd::FileDialog::new().add_filter("CSV", &["csv"]).pick_file() {
                    match load_csv(&path) {
                        Ok(t) => {
                            state.table = Some(t);
                            state.csv_path = Some(path);
                            state.cells.clear();
                        }
                        Err(e) => shared.notice(format!("Could not read {}: {e}", path.display()), true),
                    }
                }
            }
            match (&state.csv_path, &state.table) {
                (Some(path), Some(t)) => {
                    ui.label(RichText::new(path.file_name().unwrap_or_default().to_string_lossy()).strong());
                    ui.label(
                        RichText::new(format!("{} rows · columns: {}", t.rows.len(), t.headers.join(", ")))
                            .color(p.muted),
                    );
                }
                _ => {
                    ui.label(RichText::new("Columns become {{variables}} in the template.").color(p.muted));
                }
            }
        });
        if let Some(t) = &state.table {
            let missing: Vec<_> = template_vars(&template).into_iter().filter(|v| !t.headers.contains(v)).collect();
            if !missing.is_empty() {
                ui.label(
                    RichText::new(format!("The CSV has no column for: {}", missing.join(", ")))
                        .small()
                        .color(p.warning),
                );
            }
        }
    });
}

fn checks(ui: &mut Ui, state: &mut BatchState) {
    let p = palette(ui);
    theme::caption(ui, "Checks");
    theme::card(ui, |ui| {
        let mut remove = None;
        for (i, c) in state.checks.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("Output {}", c.label()));
                if ui.small_button("×").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            state.checks.remove(i);
        }
        if state.checks.is_empty() {
            ui.label(RichText::new("No checks yet: every answer without an error passes.").color(p.muted));
        }
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("check_kind").selected_text(CHECK_KINDS[state.new_kind]).show_ui(ui, |ui| {
                for (i, k) in CHECK_KINDS.iter().enumerate() {
                    ui.selectable_value(&mut state.new_kind, i, *k);
                }
            });
            let needs_value = state.new_kind != 3;
            if needs_value {
                ui.add(
                    egui::TextEdit::singleline(&mut state.new_value).desired_width(260.0).hint_text("text or pattern"),
                );
            }
            let ready = !needs_value || !state.new_value.trim().is_empty();
            if ui.add_enabled(ready, egui::Button::new("Add check")).clicked() {
                let v = std::mem::take(&mut state.new_value);
                state.checks.push(match state.new_kind {
                    0 => Check::Contains(v),
                    1 => Check::NotContains(v),
                    2 => Check::Regex(v),
                    _ => Check::ValidJson,
                });
            }
        });
        ui.label(
            RichText::new("Tip: a CSV column named \"expected\" adds a contains-check for that row.")
                .small()
                .color(p.muted),
        );
    });
}

fn run_controls(ui: &mut Ui, state: &mut BatchState, compare: &CompareState, shared: &mut Shared) {
    let p = palette(ui);
    ui.horizontal(|ui| {
        let ready = state.table.as_ref().is_some_and(|t| !t.rows.is_empty())
            && !state.template(compare).trim().is_empty()
            && !state.running;
        let label =
            format!("Run {} × {} model(s)", state.table.as_ref().map_or(0, |t| t.rows.len()), compare.targets.len());
        if ui.add_enabled(ready, egui::Button::new(RichText::new(label).strong())).clicked() {
            state.start(compare, shared);
        }
        if state.running {
            let stopping = state.cancel.is_cancelled();
            let label = if stopping { "Stopping…" } else { "Stop" };
            if ui.add_enabled(!stopping, egui::Button::new(label)).clicked() {
                state.cancel.cancel();
            }
        }
        if !state.cells.is_empty() && !state.running && ui.button("Export results…").clicked() {
            match state.export_csv() {
                Ok(csv) => save_file(shared, "promptman-batch.csv", "CSV", "csv", csv),
                Err(e) => shared.notice(format!("Could not export: {e}"), true),
            }
        }
        if state.total > 0 {
            let stopped = !state.running && state.cancel.is_cancelled() && state.done < state.total;
            let note = if stopped { " · stopped" } else { "" };
            ui.label(RichText::new(format!("{}/{} answers{note}", state.done, state.total)).color(p.muted));
        }
    });
    if state.running {
        ui.add(ProgressBar::new(state.done as f32 / state.total.max(1) as f32).desired_height(6.0).fill(p.accent));
    }
}

fn summary(ui: &mut Ui, state: &BatchState) {
    let p = palette(ui);
    theme::caption(ui, "Summary");
    theme::card(ui, |ui| {
        egui::Grid::new("batch_summary").num_columns(6).striped(true).spacing([24.0, 6.0]).show(ui, |ui| {
            for h in ["Model", "Passed", "Pass rate", "Errors", "Avg latency", "Spend"] {
                theme::caption(ui, h);
            }
            ui.end_row();
            for (slot, target) in state.targets.iter().enumerate() {
                let cells: Vec<&Cell> = state.cells.iter().filter_map(|r| r.get(slot)?.as_ref()).collect();
                let passed = cells.iter().filter(|c| c.passed).count();
                let errors = cells.iter().filter(|c| c.result.error.is_some()).count();
                let ok: Vec<f64> =
                    cells.iter().filter(|c| c.result.error.is_none()).map(|c| c.result.latency_ms).collect();
                let rate = if cells.is_empty() { 0.0 } else { passed as f64 / cells.len() as f64 };
                ui.label(RichText::new(target.to_string()).strong());
                ui.label(format!("{passed}/{}", cells.len()));
                let color = if rate >= 0.9 {
                    p.success
                } else if rate >= 0.5 {
                    p.warning
                } else {
                    p.danger
                };
                ui.label(RichText::new(format!("{:.0}%", rate * 100.0)).color(color));
                ui.label(errors.to_string());
                ui.label(if ok.is_empty() {
                    "—".into()
                } else {
                    format!("{:.0} ms", ok.iter().sum::<f64>() / ok.len() as f64)
                });
                ui.label(format!("${:.5}", cells.iter().filter_map(|c| c.result.cost_usd).sum::<f64>()));
                ui.end_row();
            }
        });
    });
}

fn grid(ui: &mut Ui, state: &BatchState) {
    let p = palette(ui);
    let Some(table) = &state.table else { return };
    theme::caption(ui, "Results");
    theme::card(ui, |ui| {
        egui::ScrollArea::horizontal().id_salt("batch_grid_scroll").show(ui, |ui| {
            egui::Grid::new("batch_grid").num_columns(2 + state.targets.len()).striped(true).spacing([20.0, 6.0]).show(
                ui,
                |ui| {
                    theme::caption(ui, "#");
                    theme::caption(ui, "Input");
                    for t in &state.targets {
                        theme::caption(ui, &t.to_string());
                    }
                    ui.end_row();
                    for (row, cells) in state.cells.iter().enumerate() {
                        ui.label(RichText::new(row.to_string()).color(p.muted));
                        let input: String = table
                            .headers
                            .iter()
                            .map(|h| table.rows[row].get(h).map_or("", |v| v))
                            .collect::<Vec<_>>()
                            .join(" | ");
                        ui.add_sized([260.0, 18.0], egui::Label::new(RichText::new(input).small()).truncate());
                        for cell in cells {
                            match cell {
                                None => {
                                    ui.label(RichText::new("…").color(p.muted));
                                }
                                Some(c) => {
                                    let (mark, color) = if c.passed { ("Pass", p.success) } else { ("Fail", p.danger) };
                                    let text = format!("{mark}  {:.0} ms", c.result.latency_ms);
                                    let hover = if c.failures.is_empty() {
                                        c.result.output.chars().take(600).collect::<String>()
                                    } else {
                                        format!(
                                            "{}\n\n{}",
                                            c.failures.join("\n"),
                                            c.result.output.chars().take(600).collect::<String>()
                                        )
                                    };
                                    ui.label(RichText::new(text).color(color)).on_hover_text(hover);
                                }
                            }
                        }
                        ui.end_row();
                    }
                },
            );
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_engine::providers::ModelRef;
    use crate::gui::test_support::{next_event, render_all, shared};

    fn table() -> CsvTable {
        // Tests run in parallel, so each call gets its own file.
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("promptman-batch-{}-{n}.csv", std::process::id()));
        std::fs::write(&path, "word,expected\nhi,hi\n\"=SUM(A1)\",x\n").unwrap();
        let t = load_csv(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        t
    }

    fn compare(shared: &Shared) -> CompareState {
        let mut c = CompareState::load(&shared.engine);
        c.targets = vec![ModelRef::new("custom", "a")];
        c.prompt = "Say {{word}}".into();
        c
    }

    #[test]
    fn every_row_runs_and_failures_are_marked() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        let (mut shared, rx) = shared();
        let compare = compare(&shared);
        let mut st =
            BatchState { table: Some(table()), checks: vec![Check::Contains("hi".into())], ..Default::default() };
        st.start(&compare, &mut shared);
        assert_eq!(st.total, 2);
        while st.running {
            st.handle(next_event(&rx), &mut shared);
        }
        assert_eq!(st.done, 2);
        let cells: Vec<&Cell> = st.cells.iter().flatten().flatten().collect();
        assert_eq!(cells.len(), 2);
        assert!(cells.iter().all(|c| !c.passed && c.failures[0].starts_with("error:")));
        let csv = st.export_csv().unwrap();
        assert_eq!(csv.lines().filter(|l| l.starts_with(char::is_numeric)).count(), 2);
        assert_eq!(shared.engine.db.history(10).unwrap().iter().filter(|h| h.kind == "batch").count(), 2);
    }

    #[test]
    fn stop_before_the_first_row_runs_nothing() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();
        let (mut shared, rx) = shared();
        let compare = compare(&shared);
        let mut st = BatchState { table: Some(table()), ..Default::default() };
        st.start(&compare, &mut shared);
        st.cancel.cancel();
        while st.running {
            st.handle(next_event(&rx), &mut shared);
        }
        assert!(st.done <= 1, "at most the row already in flight finishes");
    }

    #[test]
    fn events_from_an_older_run_are_ignored() {
        let (mut shared, _rx) = shared();
        let mut st = BatchState { token: 2, running: true, cells: vec![vec![None]], ..Default::default() };
        let cell = Cell { slot: 0, result: ResultRecord::default(), passed: true, failures: vec![] };
        st.handle(Event::BatchCell { token: 1, row: 0, cell: Box::new(cell) }, &mut shared);
        st.handle(Event::BatchDone { token: 1 }, &mut shared);
        assert!(st.running && st.cells[0][0].is_none());
        let cell = Cell { slot: 5, result: ResultRecord::default(), passed: true, failures: vec![] };
        st.handle(Event::BatchCell { token: 2, row: 9, cell: Box::new(cell) }, &mut shared);
        assert_eq!(st.done, 0, "out-of-range cells are dropped, not a crash");
    }

    #[test]
    fn renders_empty_running_and_finished() {
        let (mut shared, _rx) = shared();
        let compare = compare(&shared);
        let mut st = BatchState::default();
        render_all(|ui| show(ui, &mut st, &compare, &mut shared));
        st.table = Some(table());
        st.targets = compare.targets.clone();
        st.checks = vec![Check::Regex("^h".into()), Check::ValidJson];
        let pass = Cell {
            slot: 0,
            result: ResultRecord { output: "hi".into(), ..Default::default() },
            passed: true,
            failures: vec![],
        };
        let fail = Cell {
            slot: 0,
            result: ResultRecord { stopped: true, ..Default::default() },
            passed: false,
            failures: vec!["stopped before finishing".into()],
        };
        st.cells = vec![vec![Some(pass)], vec![Some(fail)]];
        st.running = true;
        st.done = 2;
        st.total = 2;
        render_all(|ui| show(ui, &mut st, &compare, &mut shared));
        st.running = false;
        render_all(|ui| show(ui, &mut st, &compare, &mut shared));
    }
}
