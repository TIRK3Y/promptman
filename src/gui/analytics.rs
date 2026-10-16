//! Analytics tab: spend, latency and quality over time.
use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime};
use eframe::egui::{self, Frame, Margin, ProgressBar, RichText, Ui};
use egui_plot::{Bar, BarChart, Legend, Line, Plot, PlotPoints};

use super::theme::{self, palette};
use crate::app::Shared;
use crate::core_engine::db::ModelStats;

pub struct AnalyticsState {
    days: i64,
    weekly: bool,
    /// (days, data_version) the cached queries were made for.
    cached_for: Option<(i64, u64)>,
    spend: Vec<(String, f64)>,
    stats: Vec<ModelStats>,
    latency: Vec<(String, String, f64)>,
}

impl Default for AnalyticsState {
    fn default() -> Self {
        Self { days: 30, weekly: false, cached_for: None, spend: vec![], stats: vec![], latency: vec![] }
    }
}

impl AnalyticsState {
    fn since(&self) -> NaiveDate {
        Local::now().date_naive() - Duration::days(self.days - 1)
    }

    fn refresh(&mut self, shared: &Shared) {
        if self.cached_for == Some((self.days, shared.data_version)) {
            return;
        }
        let since = self.since().format("%Y-%m-%d").to_string();
        let db = &shared.engine.db;
        self.spend = db.spend_by_day(&since).unwrap_or_default();
        self.stats = db.model_stats(&since).unwrap_or_default();
        self.latency = db.latency_points(&since).unwrap_or_default();
        self.cached_for = Some((self.days, shared.data_version));
    }

    /// Spend per bucket (day or week starting Monday), including empty buckets.
    fn buckets(&self) -> Vec<(NaiveDate, f64)> {
        let start = self.since();
        let key = |d: NaiveDate| {
            if self.weekly {
                d - Duration::days(d.weekday().num_days_from_monday() as i64)
            } else {
                d
            }
        };
        let mut out: Vec<(NaiveDate, f64)> = Vec::new();
        let mut d = start;
        let today = Local::now().date_naive();
        while d <= today {
            let k = key(d);
            if out.last().map(|(b, _)| *b) != Some(k) {
                out.push((k, 0.0));
            }
            d += Duration::days(1);
        }
        for (day, cost) in &self.spend {
            if let Ok(day) = NaiveDate::parse_from_str(day, "%Y-%m-%d") {
                if let Some(b) = out.iter_mut().find(|(b, _)| *b == key(day)) {
                    b.1 += cost;
                }
            }
        }
        out
    }
}

pub fn show(ui: &mut Ui, state: &mut AnalyticsState, shared: &mut Shared) {
    state.refresh(shared);
    let p = palette(ui);
    egui::CentralPanel::default().frame(Frame::new().fill(p.bg).inner_margin(Margin::same(16))).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| theme::title(ui, "Analytics", "Spend, speed and quality across your runs."));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if ui.button("Export all history (CSV)…").clicked() {
                        export_history(shared);
                    }
                    ui.separator();
                    for (days, label) in [(365, "1 year"), (90, "90 days"), (30, "30 days"), (7, "7 days")] {
                        ui.selectable_value(&mut state.days, days, label);
                    }
                });
            });
            ui.add_space(12.0);

            let total_spend: f64 = state.stats.iter().map(|s| s.total_cost).sum();
            let runs: usize = state.stats.iter().map(|s| s.runs).sum();
            let ok_latencies: Vec<f64> = state.latency.iter().map(|(_, _, l)| *l).collect();
            let avg_latency = if ok_latencies.is_empty() {
                "—".to_string()
            } else {
                format!("{:.0} ms", ok_latencies.iter().sum::<f64>() / ok_latencies.len() as f64)
            };
            ui.columns(4, |cols| {
                kpi(&mut cols[0], "Spend", format!("${total_spend:.4}"));
                kpi(&mut cols[1], "Model answers", runs.to_string());
                kpi(&mut cols[2], "Avg latency", avg_latency);
                kpi(&mut cols[3], "Most used", state.stats.first().map_or("—".into(), |s| s.model.clone()));
            });

            if runs == 0 {
                ui.add_space(24.0);
                ui.label(RichText::new("No runs in this period yet.").color(p.muted));
                return;
            }

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                theme::caption(ui, "Spend over time");
                ui.selectable_value(&mut state.weekly, false, "Daily");
                ui.selectable_value(&mut state.weekly, true, "Weekly");
            });
            theme::card(ui, |ui| {
                if state.buckets().iter().all(|(_, cost)| *cost == 0.0) {
                    ui.label(RichText::new("Nothing spent in this period (local models are free).").color(p.muted));
                } else {
                    spend_chart(ui, state);
                }
            });

            ui.add_space(12.0);
            theme::caption(ui, "Spend by model");
            theme::card(ui, |ui| spend_by_model(ui, &state.stats));

            ui.add_space(12.0);
            theme::caption(ui, "Latency per answer");
            theme::card(ui, |ui| {
                if state.latency.is_empty() {
                    ui.label(RichText::new("No completed answers in this period.").color(p.muted));
                } else {
                    latency_chart(ui, state);
                }
            });

            ui.add_space(12.0);
            theme::caption(ui, "Models");
            theme::card(ui, |ui| model_table(ui, &state.stats));
        });
    });
}

fn kpi(ui: &mut Ui, label: &str, value: String) {
    theme::card(ui, |ui| {
        theme::caption(ui, label);
        ui.add(egui::Label::new(RichText::new(value).size(20.0).strong()).truncate());
    });
}

fn spend_chart(ui: &mut Ui, state: &AnalyticsState) {
    let p = palette(ui);
    let buckets = state.buckets();
    let labels: Vec<String> = buckets.iter().map(|(d, _)| d.format("%b %d").to_string()).collect();
    let bars: Vec<Bar> = buckets
        .iter()
        .enumerate()
        .map(|(i, (d, cost))| Bar::new(i as f64, *cost).name(d.format("%Y-%m-%d").to_string()).fill(p.accent))
        .collect();
    Plot::new("spend_plot")
        .height(220.0)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .include_y(0.0)
        .y_axis_label("USD")
        .x_axis_formatter(move |mark, _| {
            let i = mark.value.round();
            if (mark.value - i).abs() < 1e-6 && i >= 0.0 {
                labels.get(i as usize).cloned().unwrap_or_default()
            } else {
                String::new()
            }
        })
        .show(ui, |plot| plot.bar_chart(BarChart::new("Spend", bars).width(0.7)));
}

fn spend_by_model(ui: &mut Ui, stats: &[ModelStats]) {
    let p = palette(ui);
    let mut by_cost: Vec<&ModelStats> = stats.iter().filter(|s| s.total_cost > 0.0).collect();
    by_cost.sort_by(|a, b| b.total_cost.total_cmp(&a.total_cost));
    let max = by_cost.first().map_or(0.0, |s| s.total_cost);
    if by_cost.is_empty() {
        ui.label(RichText::new("Nothing spent in this period (local models are free).").color(p.muted));
        return;
    }
    egui::Grid::new("spend_by_model").num_columns(3).spacing([16.0, 8.0]).show(ui, |ui| {
        for s in by_cost {
            ui.label(&s.model);
            ui.add(
                ProgressBar::new((s.total_cost / max) as f32).desired_width(320.0).desired_height(8.0).fill(p.accent),
            );
            ui.label(RichText::new(format!("${:.4}", s.total_cost)).monospace());
            ui.end_row();
        }
    });
}

fn latency_chart(ui: &mut Ui, state: &AnalyticsState) {
    let start = state.since().and_hms_opt(0, 0, 0).unwrap_or_default();
    let mut series: Vec<(String, Vec<[f64; 2]>)> = Vec::new();
    for (model, at, latency) in &state.latency {
        let Ok(t) = NaiveDateTime::parse_from_str(at, "%Y-%m-%d %H:%M:%S") else { continue };
        let x = (t - start).num_seconds() as f64 / 86_400.0;
        match series.iter_mut().find(|(m, _)| m == model) {
            Some((_, pts)) => pts.push([x, *latency]),
            None => series.push((model.clone(), vec![[x, *latency]])),
        }
    }
    let since = state.since();
    Plot::new("latency_plot")
        .height(240.0)
        .legend(Legend::default())
        .allow_scroll(false)
        .include_y(0.0)
        .y_axis_label("ms")
        .x_axis_formatter(move |mark, _| {
            chrono::TimeDelta::try_days(mark.value.floor() as i64)
                .and_then(|d| since.checked_add_signed(d))
                .map_or_else(String::new, |d| d.format("%b %d").to_string())
        })
        .show(ui, |plot| {
            for (model, pts) in series {
                plot.line(Line::new(model, PlotPoints::from(pts)));
            }
        });
}

fn model_table(ui: &mut Ui, stats: &[ModelStats]) {
    let p = palette(ui);
    egui::ScrollArea::horizontal().id_salt("model_table").show(ui, |ui| {
        egui::Grid::new("model_stats").num_columns(9).striped(true).spacing([20.0, 6.0]).show(ui, |ui| {
            for h in
                ["Model", "Answers", "Errors", "Avg latency", "First token", "Tok/s", "Spend", "Avg rating", "Win rate"]
            {
                theme::caption(ui, h);
            }
            ui.end_row();
            for s in stats {
                ui.label(RichText::new(&s.model).strong());
                ui.label(s.runs.to_string());
                ui.label(RichText::new(s.errors.to_string()).color(if s.errors > 0 { p.danger } else { p.text }));
                ui.label(format!("{:.0} ms", s.avg_latency_ms));
                ui.label(s.avg_ttft_ms.map_or("—".into(), |t| format!("{t:.0} ms")));
                ui.label(format!("{:.1}", s.avg_tokens_per_sec));
                ui.label(format!("${:.4}", s.total_cost));
                ui.label(s.avg_rating.map_or("—".into(), |r| format!("{r:.1} ({} rated)", s.rated)));
                ui.label(if s.contests == 0 {
                    "—".into()
                } else {
                    format!("{:.0}% of {}", 100.0 * s.wins as f64 / s.contests as f64, s.contests)
                });
                ui.end_row();
            }
        });
    });
    ui.label(
        RichText::new(
            "Win rate: in runs where 2+ answers were rated, how often this model got the single highest rating.",
        )
        .small()
        .color(p.muted),
    );
}

fn export_history(shared: &mut Shared) {
    let Some(path) =
        rfd::FileDialog::new().set_file_name("promptman-history.csv").add_filter("CSV", &["csv"]).save_file()
    else {
        return;
    };
    match shared.engine.db.export_csv(&path) {
        Ok(n) => shared.notice(format!("Exported {n} answers to {}", path.display()), false),
        Err(e) => shared.notice(format!("Could not export: {e}"), true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core_engine::db::{ResultRecord, RunRecord};
    use crate::gui::test_support::{render_all, shared};

    fn day(offset: i64) -> String {
        (Local::now().date_naive() - Duration::days(offset)).format("%Y-%m-%d").to_string()
    }

    #[test]
    fn daily_and_weekly_buckets() {
        let mut st = AnalyticsState { days: 14, ..Default::default() };
        st.spend = vec![(day(0), 1.0), (day(7), 2.0), (day(400), 99.0)];
        let daily = st.buckets();
        assert_eq!(daily.len(), 14);
        assert_eq!(daily.last().unwrap().1, 1.0);
        assert_eq!(daily.iter().map(|b| b.1).sum::<f64>(), 3.0, "spend outside the range is left out");

        st.weekly = true;
        let weekly = st.buckets();
        assert!((2..=3).contains(&weekly.len()), "{} weeks", weekly.len());
        assert!(weekly.iter().all(|(d, _)| d.weekday() == chrono::Weekday::Mon || *d == st.since()));
        assert_eq!(weekly.iter().map(|b| b.1).sum::<f64>(), 3.0);
    }

    #[test]
    fn renders_with_and_without_data() {
        let (mut shared, _rx) = shared();
        let mut st = AnalyticsState::default();
        render_all(|ui| show(ui, &mut st, &mut shared));
        let at = |offset: i64| RunRecord {
            kind: "compare".into(),
            created_at: format!("{} 10:00:00", day(offset)),
            ..Default::default()
        };
        let answer = |model: &str, cost: f64| ResultRecord {
            provider: "openai".into(),
            model: model.into(),
            cost_usd: Some(cost),
            latency_ms: 700.0,
            rating: Some(4),
            ..Default::default()
        };
        shared.engine.db.save_run(&at(0), &[answer("gpt-4o", 0.01), answer("gpt-4.1", 0.02)]).unwrap();
        shared.engine.db.save_run(&at(3), &[answer("gpt-4o", 0.03)]).unwrap();
        shared.data_version += 1;
        for days in [7, 30, 90, 365] {
            st.days = days;
            st.weekly = days > 30;
            render_all(|ui| show(ui, &mut st, &mut shared));
        }
        assert_eq!(st.stats.len(), 2);
    }
}
