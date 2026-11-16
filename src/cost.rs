//! Per-run admission control. Codex reports usage only after a turn: this is
//! NOT a provider-side hard spending cap for a running agent invocation.
use anyhow::{Result, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Debug, Serialize, clap::Args)]
pub struct Options {
    /// Explicit Codex model; no dependency on the CLI default.
    #[arg(long, default_value = "gpt-6-luna")]
    pub model: String,
    #[arg(long, default_value = "low", value_parser = ["low", "medium", "high", "xhigh"])]
    pub reasoning_effort: String,
    /// Stop admitting calls when estimated spend plus reserve exceeds this amount.
    /// Does not cap the spend of an already running Codex invocation.
    #[arg(long, default_value_t = 5.0)]
    pub max_run_cost_usd: f64,
    /// Admission reserve per invocation, not a guaranteed upper cost bound.
    #[arg(long, default_value_t = 1.0)]
    pub call_cost_reserve_usd: f64,
    #[arg(long, default_value_t = 20, value_parser=clap::value_parser!(u32).range(1..))]
    pub max_model_calls: u32,
}
impl Options {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.max_run_cost_usd.is_finite() && self.max_run_cost_usd > 0.0,
            "max-run-cost-usd must be positive and finite"
        );
        ensure!(
            self.call_cost_reserve_usd.is_finite() && self.call_cost_reserve_usd > 0.0,
            "call-cost-reserve-usd must be positive and finite"
        );
        ensure!(
            rates(&self.model).is_some(),
            "model has no verified price table; no AI calls permitted"
        );
        Ok(())
    }
    pub fn cache_identity(&self) -> String {
        format!("{}:{}", self.model, self.reasoning_effort)
    }
}

// Standard, non-regional text pricing checked 2026-11-16. For GPT-6 we use
// the long-context rate and the higher 12h cache-write rate for ALL uncached
// inputs because exec does not expose per-request lengths or cache writes.
// These conservative rates are estimates, never actual billed dollars.
// https://developers.openai.com/api/docs/pricing
// https://developers.openai.com/api/docs/models/gpt-6-luna
// https://developers.openai.com/api/docs/models/gpt-5.3-codex
fn rates(model: &str) -> Option<(f64, f64, f64)> {
    match model {
        "gpt-6-luna" => Some((0.5, 0.02, 0.75)),
        "gpt-5.3-codex" => Some((1.75, 0.175, 14.0)),
        _ => None,
    }
}
#[derive(Default, Clone, Debug, Serialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: Option<u64>,
}
fn usage(event: &Value) -> Option<Usage> {
    let u = &event["usage"];
    let result = Usage {
        input_tokens: u["input_tokens"].as_u64()?,
        cached_input_tokens: u["cached_input_tokens"].as_u64()?,
        output_tokens: u["output_tokens"].as_u64()?,
        reasoning_output_tokens: u["reasoning_output_tokens"].as_u64(),
    };
    (result.cached_input_tokens <= result.input_tokens
        && result
            .reasoning_output_tokens
            .is_none_or(|n| n <= result.output_tokens))
    .then_some(result)
}
#[derive(Serialize)]
pub struct Call {
    batch: usize,
    reason: &'static str,
    requested_model: String,
    requested_effort: String,
    // exec currently does not guarantee provider-selected identity metadata.
    observed_model: Option<String>,
    observed_effort: Option<String>,
    elapsed_seconds: f64,
    status: &'static str,
    usage: Option<Usage>,
    estimated_cost_usd: Option<f64>,
}
pub struct Meter {
    pub options: Options,
    calls: Vec<Call>,
    pub stop_reason: Option<&'static str>,
    path: PathBuf,
    review_count: usize,
}
impl Meter {
    pub fn new(options: Options, path: PathBuf) -> Result<Self> {
        options.validate()?;
        if path.exists() {
            let previous: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
            let archive = path.parent().unwrap().join("cost-history").join(format!(
                "{}.json",
                chrono::Utc::now().format("%Y%m%dT%H%M%S%.9fZ")
            ));
            crate::atomic_json(&archive, &previous)?;
        }
        let meter = Self {
            options,
            calls: vec![],
            stop_reason: None,
            path,
            review_count: 0,
        };
        meter.save()?;
        Ok(meter)
    }
    pub fn set_review_count(&mut self, count: usize) -> Result<()> {
        self.review_count = count;
        self.save()
    }
    pub fn begin(&mut self, batch: usize, reason: &'static str) -> Result<()> {
        if self.stop_reason.is_none() {
            let spent: Option<f64> = self.calls.iter().map(|c| c.estimated_cost_usd).sum();
            self.stop_reason = if self.calls.len() >= self.options.max_model_calls as usize {
                Some("model call limit reached")
            } else if spent.is_none() {
                Some("model usage unavailable; further calls stopped")
            } else if spent.unwrap() + self.options.call_cost_reserve_usd
                > self.options.max_run_cost_usd
            {
                Some("run cost admission limit reached")
            } else {
                None
            };
        }
        if let Some(reason) = self.stop_reason {
            self.save()?;
            anyhow::bail!(reason);
        }
        self.calls.push(Call {
            batch,
            reason,
            requested_model: self.options.model.clone(),
            requested_effort: self.options.reasoning_effort.clone(),
            observed_model: None,
            observed_effort: None,
            elapsed_seconds: 0.0,
            status: "started",
            usage: None,
            estimated_cost_usd: None,
        });
        // Persist BEFORE spawn: an interrupted call must never appear free.
        self.save()
    }
    pub fn finish(&mut self, events: &str, elapsed: f64, success: bool) -> Result<()> {
        let call = self.calls.last_mut().expect("begin before finish");
        call.elapsed_seconds = elapsed;
        call.status = if success { "completed" } else { "failed" };
        let mut completions = vec![];
        let mut provider_stop = false;
        for line in events.lines() {
            let Ok(event) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            match event["type"].as_str() {
                Some("turn.completed") => completions.push(usage(&event)),
                Some("error" | "turn.failed") => {
                    // Inspect only structured error events, never untrusted tool output.
                    let error = event.to_string().to_ascii_lowercase();
                    provider_stop |= [
                        "quota",
                        "429",
                        "rate limit",
                        "rate_limit",
                        "insufficient",
                        "billing",
                        "credit balance",
                    ]
                    .iter()
                    .any(|s| error.contains(s));
                }
                _ => {}
            }
        }
        // exec handles exactly one turn. Ambiguous/repeated summaries cannot be
        // safely added or deduplicated without request IDs; fail closed.
        call.usage = if completions.len() == 1 {
            completions.pop().flatten()
        } else {
            None
        };
        if success && let Some(u) = &call.usage {
            let (input, cached, output) = rates(&call.requested_model).unwrap();
            call.estimated_cost_usd = Some(
                ((u.input_tokens - u.cached_input_tokens) as f64 * input
                    + u.cached_input_tokens as f64 * cached
                    + u.output_tokens as f64 * output)
                    / 1_000_000.0,
            );
        }
        self.stop_reason = if provider_stop {
            Some("provider quota or rate limit; further calls stopped")
        } else if call.estimated_cost_usd.is_none() {
            Some("model usage unavailable; further calls stopped")
        } else if self
            .calls
            .iter()
            .filter_map(|c| c.estimated_cost_usd)
            .sum::<f64>()
            >= self.options.max_run_cost_usd
        {
            Some("run cost admission limit reached")
        } else {
            None
        };
        self.save()
    }
    pub fn summary(&self) -> Value {
        let estimated: Option<f64> = self.calls.iter().map(|c| c.estimated_cost_usd).sum();
        let mut retries = BTreeMap::<&str, usize>::new();
        let mut totals = Usage::default();
        let mut reasoning = Some(0u64);
        for c in &self.calls {
            if c.reason != "initial" {
                *retries.entry(c.reason).or_default() += 1;
            }
            if let Some(u) = &c.usage {
                totals.input_tokens += u.input_tokens;
                totals.cached_input_tokens += u.cached_input_tokens;
                totals.output_tokens += u.output_tokens;
                reasoning = reasoning.zip(u.reasoning_output_tokens).map(|(a, b)| a + b);
            } else {
                reasoning = None;
            }
        }
        totals.reasoning_output_tokens = reasoning;
        let mut by_model = BTreeMap::new();
        if !self.calls.is_empty() {
            by_model.insert(&self.options.model, &totals);
        }
        json!({"pricing_version":"2026-11-16-standard-conservative-v1",
            "pricing_basis":"requested_model; conservative long-context/cache-write rates; no regional or fast-mode surcharge",
            "settings":self.options,"estimated_cost_usd":estimated,
            "known_cost_usd":self.calls.iter().filter_map(|c| c.estimated_cost_usd).sum::<f64>(),
            "usage_by_model":by_model,"usage_complete":self.calls.iter().all(|c| c.estimated_cost_usd.is_some()),
            "codex_invocations":self.calls.len(),"review_count":self.review_count,
            "calls_per_review":if self.review_count > 0 {Some(self.calls.len() as f64 / self.review_count as f64)} else {None},
            "cache_hit_rate":if totals.input_tokens > 0 {Some(totals.cached_input_tokens as f64 / totals.input_tokens as f64)} else {None},
            "retries_by_reason":retries,"stop_reason":self.stop_reason,
            "billing_reconciliation":"unavailable","calls":self.calls})
    }
    fn save(&self) -> Result<()> {
        crate::atomic_json(&self.path, &self.summary())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn meter(path: PathBuf) -> Meter {
        Meter::new(
            Options {
                model: "gpt-6-luna".into(),
                reasoning_effort: "low".into(),
                max_run_cost_usd: 5.0,
                call_cost_reserve_usd: 1.0,
                max_model_calls: 2,
            },
            path,
        )
        .unwrap()
    }
    #[test]
    fn malformed_or_duplicate_usage_is_unknown() {
        for events in [
            r#"{"type":"turn.completed","usage":{"input_tokens":1,"cached_input_tokens":2,"output_tokens":0}}"#,
            "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":1,\"cached_input_tokens\":0,\"output_tokens\":0}}\n{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":1,\"cached_input_tokens\":0,\"output_tokens\":0}}",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut m = meter(dir.path().join("cost.json"));
            m.begin(1, "initial").unwrap();
            m.finish(events, 1.0, true).unwrap();
            assert!(m.summary()["estimated_cost_usd"].is_null());
            assert!(m.begin(2, "initial").is_err());
        }
    }

    #[test]
    fn unknown_usage_is_not_zero_and_stops_retries() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = meter(dir.path().join("cost.json"));
        m.begin(1, "initial").unwrap();
        m.finish("", 1.0, true).unwrap();
        assert!(m.summary()["estimated_cost_usd"].is_null());
        assert!(m.begin(2, "discussion").is_err());
    }
    #[test]
    fn cached_and_reasoning_tokens_are_not_double_charged() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = meter(dir.path().join("cost.json"));
        m.begin(1, "initial").unwrap();
        m.finish(r#"{"type":"turn.completed","usage":{"input_tokens":1000000,"cached_input_tokens":500000,"output_tokens":1000000,"reasoning_output_tokens":500000}}"#, 1.0, true).unwrap();
        assert_eq!(m.summary()["estimated_cost_usd"], json!(1.01));
        assert_eq!(m.summary()["cache_hit_rate"], json!(0.5));
        m.begin(1, "repair").unwrap();
        m.finish(r#"{"type":"turn.completed","usage":{"input_tokens":0,"cached_input_tokens":0,"output_tokens":0}}"#, 1.0, true).unwrap();
        assert!(m.summary()["usage_by_model"]["gpt-6-luna"]["reasoning_output_tokens"].is_null());
        assert!(m.begin(2, "initial").is_err());
    }
    #[test]
    fn error_text_in_tool_output_cannot_trigger_quota_detection() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = meter(dir.path().join("cost.json"));
        m.begin(1, "initial").unwrap();
        m.finish("{\"type\":\"item.completed\",\"item\":{\"aggregated_output\":\"429 quota exceeded\"}}\n{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":1,\"cached_input_tokens\":0,\"output_tokens\":1}}", 1.0, true).unwrap();
        assert!(m.stop_reason.is_none());
    }
}
