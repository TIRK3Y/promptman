//! Runs prompts against models and records the results. Shared by the GUI and CLI.
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::db::{Db, ResultRecord, RunRecord};
use super::evals::{clean_rewrite, optimize_request, OPTIMIZE_SYSTEM};
use super::hardware::Sampler;
use super::llm::{self, Endpoint, Message, Params, Prompt};
use super::providers::{provider, ModelRef};
use super::secrets;
use super::token_auditor::{count_tokens, Pricing};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Budget {
    pub monthly_cap_usd: f64,
    pub warn_percent: f32,
    /// Block cloud runs once the monthly cap is reached.
    pub hard_stop: bool,
}

impl Default for Budget {
    fn default() -> Self {
        Self { monthly_cap_usd: 10.0, warn_percent: 80.0, hard_stop: false }
    }
}

pub struct Engine {
    pub db: Db,
    pub client: Client,
    pub pricing: RwLock<Pricing>,
    pub data_dir: PathBuf,
}

/// Called with (slot, text fragment) while answers stream in.
pub type DeltaFn = Arc<dyn Fn(usize, &str) + Send + Sync>;
/// Called as soon as one model's answer is complete (or stopped, or failed).
pub type DoneFn = Arc<dyn Fn(&ResultRecord) + Send + Sync>;

/// What a multi-model run reports while it's in progress.
pub struct RunHooks {
    pub on_delta: DeltaFn,
    pub on_done: DoneFn,
}

impl RunHooks {
    pub fn none() -> Self {
        Self { on_delta: Arc::new(|_, _| {}), on_done: Arc::new(|_| {}) }
    }
}

impl Engine {
    /// Opens the data folder, migrating pre-1.0 files. Returns notes to show the user.
    pub fn open() -> Result<(Self, Vec<String>), String> {
        // PROMPTMAN_DATA_DIR gives a separate profile (portable installs, testing, CI).
        let data_dir = std::env::var_os("PROMPTMAN_DATA_DIR")
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                directories::ProjectDirs::from("com", "promptman", "PromptMan").map(|d| d.data_dir().to_path_buf())
            })
            .unwrap_or_else(|| std::env::temp_dir().join("PromptMan"));
        std::fs::create_dir_all(&data_dir).map_err(|e| format!("can't create {}: {e}", data_dir.display()))?;

        let db = Db::open(&data_dir.join("promptman.db"))?;
        let mut notes = Vec::new();
        match db.import_legacy_ledger(&data_dir) {
            Ok(0) => {}
            Ok(n) => notes.push(format!("Imported {n} records from the old usage ledger.")),
            Err(e) => notes.push(format!("Could not import the old usage ledger: {e}")),
        }
        match secrets::migrate_legacy_vault(&data_dir) {
            Ok(0) => {}
            Ok(n) => notes.push(format!("Moved {n} API key(s) into the OS keychain and deleted the old key file.")),
            Err(e) => notes.push(format!("Could not move API keys into the OS keychain: {e}")),
        }
        let (pricing, pricing_note) = Pricing::load_or_default(&data_dir);
        notes.extend(pricing_note);
        let client = Client::builder()
            // No total timeout: long replies are fine while data keeps arriving.
            // Stalls are caught per chunk (llm::IDLE_TIMEOUT).
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| e.to_string())?;
        Ok((Self { db, client, pricing: RwLock::new(pricing), data_dir }, notes))
    }

    /// An engine on an in-memory database whose OpenAI-compatible "custom" endpoint is `custom_base`.
    #[cfg(test)]
    pub(crate) fn for_test(custom_base: &str) -> Self {
        let db = Db::open_in_memory().expect("in-memory database");
        db.set_setting("base_url.custom", &custom_base.to_string()).expect("setting");
        let pricing = Pricing { path: PathBuf::new(), entries: super::token_auditor::default_prices() };
        Self { db, client: Client::new(), pricing: RwLock::new(pricing), data_dir: std::env::temp_dir() }
    }

    pub fn reload_pricing(&self) -> Result<(), String> {
        let fresh = Pricing::load(&self.data_dir)?;
        *self.pricing.write().unwrap_or_else(|p| p.into_inner()) = fresh;
        Ok(())
    }

    pub fn base_url(&self, provider_id: &str) -> String {
        self.db
            .get_setting::<String>(&format!("base_url.{provider_id}"))
            .filter(|u| !u.trim().is_empty())
            .or_else(|| provider(provider_id).map(|p| p.default_base_url.to_string()))
            .unwrap_or_default()
    }

    pub fn set_base_url(&self, provider_id: &str, url: &str) -> Result<(), String> {
        self.db.set_setting(&format!("base_url.{provider_id}"), &url.trim().to_string())
    }

    pub fn budget(&self) -> Budget {
        self.db.get_setting("budget").unwrap_or_default()
    }

    pub fn month_spend(&self) -> f64 {
        let month_start = chrono::Local::now().format("%Y-%m-01").to_string();
        self.db.spend_since(&month_start).unwrap_or(0.0)
    }

    /// True when the hard stop is on and this month's cap is used up.
    pub fn cloud_blocked(&self) -> bool {
        let b = self.budget();
        b.hard_stop && self.month_spend() >= b.monthly_cap_usd
    }

    fn endpoint(&self, m: &ModelRef) -> Result<Endpoint, String> {
        let info = provider(&m.provider).ok_or_else(|| format!("unknown provider {}", m.provider))?;
        let api_key = secrets::get_key(info.id);
        if info.needs_key && api_key.is_none() {
            return Err(format!("No API key for {}. Add one in Settings.", info.name));
        }
        Ok(Endpoint {
            provider_id: info.id.to_string(),
            protocol: info.protocol,
            base_url: self.base_url(info.id),
            api_key,
            idle_timeout: llm::IDLE_TIMEOUT,
        })
    }

    /// Runs one model and returns its result (errors are recorded, not raised).
    pub async fn run_one(
        &self,
        slot: usize,
        m: &ModelRef,
        prompt: Prompt<'_>,
        cancel: &CancellationToken,
        on_delta: impl FnMut(&str),
    ) -> ResultRecord {
        let Prompt { system, messages, .. } = prompt;
        let mut rec = ResultRecord {
            slot,
            provider: m.provider.clone(),
            model: m.model.clone(),
            is_local: m.is_local(),
            ..Default::default()
        };
        if !m.is_local() && self.cloud_blocked() {
            rec.error = Some("Monthly budget reached. Raise the cap or turn off the hard stop in Settings.".into());
            return rec;
        }
        let ep = match self.endpoint(m) {
            Ok(ep) => ep,
            Err(e) => {
                rec.error = Some(e);
                return rec;
            }
        };

        let sampler = m.is_local().then(Sampler::start);
        let start = Instant::now();
        let outcome = llm::stream_chat(&self.client, &ep, &m.model, &prompt, cancel, on_delta).await;
        rec.latency_ms = start.elapsed().as_secs_f64() * 1000.0;
        if let Some(s) = sampler {
            // Joining the sampler thread can take up to half a second; keep it off the async workers.
            let hw = tokio::task::spawn_blocking(move || s.finish()).await.unwrap_or_default();
            rec.cpu_peak = Some(hw.cpu_percent);
            rec.ram_peak_mb = Some(hw.ram_used_mb);
            rec.gpu_peak = hw.gpu_percent;
        }

        let (c, ended_early) = match outcome {
            Ok(c) => (c, None),
            // Nothing reached the model, so nothing was billed.
            Err(e) if !e.started => {
                rec.stopped = e.cancelled;
                if e.cancelled {
                    rec.cost_usd = Some(0.0);
                    rec.note = Some("Stopped before the model replied".into());
                } else {
                    rec.error = Some(e.message);
                }
                return rec;
            }
            // The model was working: keep its text and count what it probably cost.
            Err(e) => {
                rec.stopped = e.cancelled;
                let why = if e.cancelled { "Stopped by you" } else { "Interrupted" };
                if !e.cancelled {
                    rec.error = Some(e.message);
                }
                (e.partial, Some(why))
            }
        };
        rec.output = c.text;
        rec.ttft_ms = c.ttft_ms;
        rec.note = match ended_early {
            Some(why) => Some(format!("{why}; tokens and cost are estimated from the text received")),
            None => c.note,
        };
        rec.tokens_estimated = c.prompt_tokens.is_none() || c.completion_tokens.is_none();
        rec.prompt_tokens = c.prompt_tokens.unwrap_or_else(|| {
            let input: String = std::iter::once(system).chain(messages.iter().map(|m| m.content.as_str())).collect();
            count_tokens(&input, &m.model).0
        });
        rec.completion_tokens = c.completion_tokens.unwrap_or_else(|| count_tokens(&rec.output, &m.model).0);

        // Generation speed excludes the wait for the first token.
        let gen_secs = (rec.latency_ms - rec.ttft_ms.unwrap_or(0.0)) / 1000.0;
        rec.tokens_per_sec = if gen_secs > 0.0 { rec.completion_tokens as f64 / gen_secs } else { 0.0 };

        let pricing = self.pricing.read().unwrap_or_else(|p| p.into_inner());
        rec.cost_usd = pricing.cost(m, rec.prompt_tokens, rec.completion_tokens);
        if rec.cost_usd.is_none() {
            let note = format!("No price for {m} in pricing.json; cost not counted");
            rec.note = Some(rec.note.map_or(note.clone(), |n| format!("{n}. {note}")));
        }
        rec
    }

    /// Rewrites a draft into a clearer, shorter prompt. The call is recorded so its cost counts.
    pub async fn optimize_prompt(&self, model: &ModelRef, draft: &str) -> Result<String, String> {
        let params = Params { temperature: 0.2, top_p: 1.0, max_tokens: 8000 };
        let request = optimize_request(draft);
        let never = CancellationToken::new();
        let messages = [Message::user(request.clone())];
        let rec = self
            .run_one(0, model, Prompt { system: OPTIMIZE_SYSTEM, messages: &messages, params: &params }, &never, |_| {})
            .await;
        let run = RunRecord {
            kind: "optimize".into(),
            prompt: request,
            system: OPTIMIZE_SYSTEM.into(),
            params,
            ..Default::default()
        };
        let _ = self.db.save_run(&run, std::slice::from_ref(&rec));
        if let Some(e) = rec.error {
            return Err(e);
        }
        Some(clean_rewrite(&rec.output))
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "The model returned an empty rewrite".into())
    }

    /// Runs several models in parallel. `conversations[i]` is the message list for target `i`,
    /// and cancelling `cancels[i]` stops that target (keeping its partial output).
    pub async fn run_all(
        self: &Arc<Self>,
        targets: &[ModelRef],
        system: &str,
        conversations: Vec<Vec<Message>>,
        params: &Params,
        cancels: Vec<CancellationToken>,
        hooks: RunHooks,
    ) -> Vec<ResultRecord> {
        let RunHooks { on_delta, on_done } = hooks;
        let tasks: Vec<_> = targets
            .iter()
            .cloned()
            .zip(conversations)
            .zip(cancels)
            .enumerate()
            .map(|(slot, ((target, messages), cancel))| {
                let engine = Arc::clone(self);
                let system = system.to_string();
                let params = params.clone();
                let on_delta = Arc::clone(&on_delta);
                let on_done = Arc::clone(&on_done);
                tokio::spawn(async move {
                    let prompt = Prompt { system: &system, messages: &messages, params: &params };
                    let rec = engine.run_one(slot, &target, prompt, &cancel, |d| on_delta(slot, d)).await;
                    on_done(&rec);
                    rec
                })
            })
            .collect();

        let mut results = Vec::with_capacity(tasks.len());
        for (slot, task) in tasks.into_iter().enumerate() {
            results.push(task.await.unwrap_or_else(|e| ResultRecord {
                slot,
                provider: targets[slot].provider.clone(),
                model: targets[slot].model.clone(),
                error: Some(format!("task failed: {e}")),
                ..Default::default()
            }));
        }
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::core_engine::db::RunRecord;
    use crate::core_engine::llm::test_server::{serve, sse, Step, END, OK_HEAD};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn engine_for(base: &str) -> Engine {
        Engine::for_test(base)
    }

    async fn ask(engine: &Engine, cancel: &CancellationToken) -> ResultRecord {
        let messages = [Message::user("Say hello")];
        let prompt = Prompt { system: "", messages: &messages, params: &Params::default() };
        engine.run_one(0, &ModelRef::new("custom", "m"), prompt, cancel, |_| {}).await
    }

    #[tokio::test]
    async fn stopped_reply_keeps_text_and_estimates_tokens() {
        let base =
            serve(vec![vec![Step::Send(OK_HEAD.into()), Step::Send(sse("Hello there")), Step::Wait(3000)]]).await;
        let engine = engine_for(&base);
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            trigger.cancel();
        });
        let rec = ask(&engine, &cancel).await;
        assert!(rec.stopped);
        assert!(rec.error.is_none());
        assert_eq!(rec.output, "Hello there");
        assert!(rec.tokens_estimated && rec.prompt_tokens > 0 && rec.completion_tokens > 0);
        assert!(rec.note.unwrap().starts_with("Stopped by you"));
        assert_eq!(rec.cost_usd, Some(0.0)); // the custom endpoint counts as local
    }

    #[tokio::test]
    async fn interrupted_reply_keeps_text_and_reports_the_error() {
        let base = serve(vec![vec![Step::Send(OK_HEAD.into()), Step::Send(sse("Partial"))]]).await;
        let engine = engine_for(&base);
        let rec = ask(&engine, &CancellationToken::new()).await;
        assert!(!rec.stopped);
        assert!(rec.error.unwrap().contains("Connection lost"));
        assert_eq!(rec.output, "Partial");
        assert!(rec.completion_tokens > 0);
        assert!(rec.note.unwrap().starts_with("Interrupted"));
    }

    fn spend(engine: &Engine, created_at: &str, cost: f64) {
        let run = RunRecord { kind: "compare".into(), created_at: created_at.into(), ..Default::default() };
        let result = ResultRecord {
            provider: "openai".into(),
            model: "gpt-4o".into(),
            cost_usd: Some(cost),
            ..Default::default()
        };
        engine.db.save_run(&run, &[result]).unwrap();
    }

    #[tokio::test]
    async fn hard_stop_blocks_cloud_models_without_any_request() {
        let engine = engine_for("http://127.0.0.1:9/v1");
        engine.db.set_setting("budget", &Budget { monthly_cap_usd: 1.0, warn_percent: 80.0, hard_stop: true }).unwrap();
        spend(&engine, "", 1.5);
        assert!(engine.cloud_blocked());
        let messages = [Message::user("hi")];
        let prompt = Prompt { system: "", messages: &messages, params: &Params::default() };
        let rec =
            engine.run_one(0, &ModelRef::new("openai", "gpt-4o"), prompt, &CancellationToken::new(), |_| {}).await;
        assert!(rec.error.unwrap().contains("Monthly budget reached"));
        assert_eq!(rec.latency_ms, 0.0, "no request should have been made");
    }

    #[test]
    fn budget_only_blocks_with_hard_stop_and_only_this_month() {
        let engine = engine_for("http://127.0.0.1:9/v1");
        spend(&engine, "2000-01-01 00:00:00", 50.0); // an old month never counts
        assert_eq!(engine.month_spend(), 0.0);
        spend(&engine, "", 20.0);
        assert!((engine.month_spend() - 20.0).abs() < 1e-9);
        assert!(!engine.cloud_blocked(), "default budget has no hard stop");
        engine.db.set_setting("budget", &Budget { hard_stop: true, ..Budget::default() }).unwrap();
        assert!(engine.cloud_blocked());
    }

    #[test]
    fn base_url_defaults_and_overrides() {
        let engine = engine_for("http://example.test/v1");
        assert_eq!(engine.base_url("custom"), "http://example.test/v1");
        assert_eq!(engine.base_url("openai"), "https://api.openai.com/v1");
        engine.set_base_url("openai", "  https://proxy.test/v1  ").unwrap();
        assert_eq!(engine.base_url("openai"), "https://proxy.test/v1");
        engine.set_base_url("openai", "").unwrap();
        assert_eq!(engine.base_url("openai"), "https://api.openai.com/v1", "blank falls back to the default");
    }

    #[tokio::test]
    async fn run_all_keeps_slot_order_and_reports_each_model() {
        let ok = |t: &str| vec![Step::Send(OK_HEAD.into()), Step::Send(sse(t)), Step::Send(END.into())];
        let base = serve(vec![ok("one"), ok("two")]).await;
        let engine = Arc::new(engine_for(&base));
        let done = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&done);
        let hooks = RunHooks {
            on_delta: Arc::new(|_, _| {}),
            on_done: Arc::new(move |_| {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
        };
        let targets = [ModelRef::new("custom", "a"), ModelRef::new("custom", "b")];
        let convs = vec![vec![Message::user("hi")]; 2];
        let cancels = vec![CancellationToken::new(), CancellationToken::new()];
        let results = engine.run_all(&targets, "", convs, &Params::default(), cancels, hooks).await;
        assert_eq!(done.load(Ordering::SeqCst), 2);
        assert_eq!(results.iter().map(|r| r.slot).collect::<Vec<_>>(), vec![0, 1]);
        assert_eq!(results[0].model, "a");
        let mut outputs: Vec<_> = results.iter().map(|r| r.output.as_str()).collect();
        outputs.sort();
        assert_eq!(outputs, vec!["one", "two"]);
        assert!(results.iter().all(|r| r.error.is_none() && r.cost_usd == Some(0.0)));
    }

    #[tokio::test]
    async fn run_all_can_stop_one_model_before_it_starts() {
        let base = serve(vec![vec![Step::Send(OK_HEAD.into()), Step::Send(sse("ok")), Step::Send(END.into())]]).await;
        let engine = Arc::new(engine_for(&base));
        let stopped = CancellationToken::new();
        stopped.cancel();
        let targets = [ModelRef::new("custom", "a"), ModelRef::new("custom", "b")];
        let convs = vec![vec![Message::user("hi")]; 2];
        let results = engine
            .run_all(&targets, "", convs, &Params::default(), vec![CancellationToken::new(), stopped], RunHooks::none())
            .await;
        assert_eq!(results[0].output, "ok");
        assert!(results[1].stopped && results[1].error.is_none());
        assert_eq!(results[1].cost_usd, Some(0.0), "nothing was sent, so nothing was billed");
        assert_eq!(results[1].note.as_deref(), Some("Stopped before the model replied"));
    }

    #[tokio::test]
    async fn optimize_prompt_returns_clean_text_and_records_the_call() {
        let base = serve(vec![vec![
            Step::Send(OK_HEAD.into()),
            Step::Send(sse("Summarize {{text}}.")),
            Step::Send(END.into()),
        ]])
        .await;
        let engine = engine_for(&base);
        let rewritten =
            engine.optimize_prompt(&ModelRef::new("custom", "m"), "pls summarize {{text}} thx").await.unwrap();
        assert_eq!(rewritten, "Summarize {{text}}.");
        let history = engine.db.history(10).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].kind, "optimize");
    }

    #[tokio::test]
    async fn optimize_prompt_reports_failures() {
        let engine = engine_for("http://127.0.0.1:9/v1");
        let err = engine.optimize_prompt(&ModelRef::new("custom", "m"), "draft").await.unwrap_err();
        assert!(err.contains("Can't reach"), "{err}");
    }

    #[test]
    fn data_dir_comes_from_the_environment() {
        let dir = std::env::temp_dir().join(format!("promptman-datadir-{}", std::process::id()));
        // Only this test reads or writes PROMPTMAN_DATA_DIR.
        std::env::set_var("PROMPTMAN_DATA_DIR", &dir);
        let opened = Engine::open();
        std::env::remove_var("PROMPTMAN_DATA_DIR");
        let (engine, notes) = opened.unwrap();
        assert_eq!(engine.data_dir, dir);
        assert!(dir.join("promptman.db").exists() && dir.join("pricing.json").exists());
        assert!(notes.is_empty(), "{notes:?}");
        drop(engine);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn budget_tolerates_missing_and_unknown_fields() {
        let b: Budget = serde_json::from_str(r#"{"monthly_cap_usd": 25.0, "added_later": 1}"#).unwrap();
        assert_eq!(b, Budget { monthly_cap_usd: 25.0, ..Budget::default() });
    }
}
