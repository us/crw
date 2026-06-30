//! `crw bench` — reproducible search-quality benchmark harness.
//!
//! Runs a QA dataset (FRAMES) through a [`SearchProvider`] (crw's `/v1/search`
//! answer path) and grades each answer with an LLM judge, then writes a
//! snapshot to `bench/runs/<unixts>/` (results jsonl + report json/md) so a
//! run is reproducible and diffable across code changes.
//!
//! This is a **local/release tool, never a CI gate**: it needs a running crw
//! server (with SearXNG + an LLM for the answer path), an LLM key for the
//! judge, and network access to fetch the dataset — none of which exist in CI.

use clap::Args;
use crw_core::config::{AppConfig, LlmConfig};
use crw_extract::llm;
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand::seq::IndexedRandom;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::teardown::CmdError;

#[derive(Args)]
pub struct BenchArgs {
    /// Dataset to run. `frames` auto-downloads google/frames-benchmark.
    #[arg(long, default_value = "frames")]
    pub dataset: String,

    /// Use a local TSV/JSONL dataset file instead of downloading. TSV must have
    /// `Prompt` + `Answer` columns; JSONL objects must have `prompt`/`answer`
    /// (or `Prompt`/`Answer`) keys.
    #[arg(long)]
    pub dataset_file: Option<PathBuf>,

    /// Base URL of the running crw server under test.
    #[arg(long, default_value = "http://localhost:3000")]
    pub server: String,

    /// Bearer key for the server under test, if it requires auth.
    #[arg(long, env = "CRW_API_KEY")]
    pub api_key: Option<String>,

    /// Cap the number of questions (0 = all).
    #[arg(long, default_value_t = 0)]
    pub limit: usize,

    /// Number of search results the answer leg may draw from.
    #[arg(long, default_value_t = 10)]
    pub search_limit: u32,

    /// Judge model — overrides the configured `extraction.llm` model.
    #[arg(long)]
    pub judge_model: Option<String>,

    /// Output directory root for run snapshots.
    #[arg(long, default_value = "bench/runs")]
    pub output: PathBuf,

    /// Per-request timeout (seconds) to the server under test.
    #[arg(long, default_value_t = 120)]
    pub timeout_secs: u64,

    /// RNG seed for the bootstrap CI, so the reported interval is reproducible.
    #[arg(long, default_value_t = 42)]
    pub seed: u64,
}

/// One graded question.
#[derive(Debug, Clone)]
struct QaItem {
    prompt: String,
    answer: String,
}

/// Per-item run record (one line of `frames_results.jsonl`).
#[derive(Debug, Serialize)]
struct ItemResult {
    prompt: String,
    truth: String,
    prediction: String,
    passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Aggregate run report (`report.json`).
#[derive(Debug, Serialize)]
struct Report {
    dataset: String,
    provider: String,
    server: String,
    judge_model: String,
    n: usize,
    passed: usize,
    score: f64,
    ci_low: f64,
    ci_high: f64,
    seed: u64,
    timestamp_unix: u64,
}

/// A thing the bench can ask a question and get back a synthesized answer.
/// One impl today ([`CrwHttp`]); the trait is the seam where a Brave/Tavily/
/// reference provider drops in for head-to-head runs.
#[allow(async_fn_in_trait)] // private trait, static dispatch only — no async-trait dep needed
trait SearchProvider {
    async fn answer(&self, query: &str) -> Result<String, String>;
    fn name(&self) -> &str;
}

/// Posts `/v1/search` with `answer:true` and returns the synthesized answer.
struct CrwHttp {
    client: reqwest::Client,
    base: String,
    key: Option<String>,
    search_limit: u32,
}

impl SearchProvider for CrwHttp {
    async fn answer(&self, query: &str) -> Result<String, String> {
        // Minimal local view of the envelope so the bench stays decoupled from
        // crw-core's full SearchResponseData shape.
        #[derive(Deserialize)]
        struct Envelope {
            data: Option<Data>,
        }
        #[derive(Deserialize)]
        struct Data {
            answer: Option<String>,
        }

        let mut req = self
            .client
            .post(format!("{}/v1/search", self.base.trim_end_matches('/')))
            .json(&serde_json::json!({
                "query": query,
                "answer": true,
                "limit": self.search_limit,
            }));
        if let Some(k) = &self.key {
            req = req.bearer_auth(k);
        }
        let resp = req.send().await.map_err(|e| format!("request: {e}"))?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| format!("body: {e}"))?;
        if !status.is_success() {
            return Err(format!(
                "HTTP {status}: {}",
                body.chars().take(200).collect::<String>()
            ));
        }
        let env: Envelope = serde_json::from_str(&body).map_err(|e| format!("decode: {e}"))?;
        Ok(env.data.and_then(|d| d.answer).unwrap_or_default())
    }

    fn name(&self) -> &str {
        "crw"
    }
}

pub async fn run(args: BenchArgs) -> Result<(), CmdError> {
    if let Err(e) = run_inner(args).await {
        eprintln!("bench error: {e}");
        return Err(CmdError::code_only(1));
    }
    Ok(())
}

async fn run_inner(args: BenchArgs) -> Result<(), String> {
    // ── Judge config: configured extraction.llm, model overridden, temp 0 so a
    // real quality lever is distinguishable from sampling noise. ──
    let app_config = AppConfig::load().unwrap_or_default();
    let mut judge_cfg: LlmConfig = app_config.extraction.llm.ok_or_else(|| {
        "bench judge requires an LLM — set CRW_EXTRACTION__LLM__API_KEY (and model)".to_string()
    })?;
    if let Some(m) = &args.judge_model {
        judge_cfg.model = m.clone();
    }
    judge_cfg.temperature = Some(0.0);
    if judge_cfg.api_key.is_empty() {
        return Err("bench judge requires a non-empty LLM api_key".to_string());
    }

    // ── Dataset ──
    let dataset_path = ensure_dataset(&args).await?;
    let mut items = load_dataset(&dataset_path)?;
    if args.limit > 0 && items.len() > args.limit {
        items.truncate(args.limit);
    }
    if items.is_empty() {
        return Err(format!(
            "no questions loaded from {}",
            dataset_path.display()
        ));
    }
    eprintln!(
        "bench: {} questions from {} → server {} (judge {})",
        items.len(),
        dataset_path.display(),
        args.server,
        judge_cfg.model
    );

    // ── Run ──
    let provider = CrwHttp {
        client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(args.timeout_secs))
            .build()
            .map_err(|e| format!("http client: {e}"))?,
        base: args.server.clone(),
        key: args.api_key.clone(),
        search_limit: args.search_limit,
    };

    // ponytail: sequential — one question at a time. A bench run is offline and
    // correctness/clarity beat wall-clock; add --concurrency if 800 questions
    // is too slow in practice.
    let mut results = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let (prediction, mut err) = match provider.answer(&item.prompt).await {
            Ok(a) => (a, None),
            Err(e) => (String::new(), Some(e)),
        };
        let passed = if prediction.is_empty() {
            false
        } else {
            match judge(&judge_cfg, &item.prompt, &item.answer, &prediction).await {
                Ok(p) => p,
                Err(e) => {
                    err = Some(format!("judge: {e}"));
                    false
                }
            }
        };
        results.push(ItemResult {
            prompt: item.prompt.clone(),
            truth: item.answer.clone(),
            prediction,
            passed,
            error: err,
        });
        if (i + 1) % 10 == 0 || i + 1 == items.len() {
            let p = results.iter().filter(|r| r.passed).count();
            eprintln!("  {}/{} done · {} pass", i + 1, items.len(), p);
        }
    }

    // ── Aggregate + snapshot ──
    let passed = results.iter().filter(|r| r.passed).count();
    let n = results.len();
    let score = passed as f64 / n as f64;
    let (ci_low, ci_high) = bootstrap_ci(&results, args.seed);
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let report = Report {
        dataset: args.dataset.clone(),
        provider: provider.name().to_string(),
        server: args.server.clone(),
        judge_model: judge_cfg.model.clone(),
        n,
        passed,
        score,
        ci_low,
        ci_high,
        seed: args.seed,
        timestamp_unix: ts,
    };

    let run_dir = args.output.join(ts.to_string());
    write_snapshot(&run_dir, &report, &results)?;

    println!(
        "\n{} {}/{} = {:.1}% (95% CI {:.1}–{:.1}%)\n→ {}",
        report.dataset,
        passed,
        n,
        score * 100.0,
        ci_low * 100.0,
        ci_high * 100.0,
        run_dir.display()
    );
    Ok(())
}

/// LLM judge: PASS if the prediction answers the question per the ground truth.
async fn judge(
    cfg: &LlmConfig,
    question: &str,
    truth: &str,
    prediction: &str,
) -> Result<bool, String> {
    let sys = "You are a strict grader for a question-answering benchmark. Given a QUESTION, \
        the GROUND TRUTH answer, and a model PREDICTION, decide whether the prediction is \
        correct. It is correct if it contains the ground-truth answer or an equivalent (same \
        entity/value, wording may differ). Extra correct detail is fine; a wrong, missing, or \
        contradicted answer is incorrect. Reply with EXACTLY one word: PASS or FAIL.";
    let user = format!(
        "QUESTION:\n{question}\n\nGROUND TRUTH:\n{truth}\n\nPREDICTION:\n{prediction}\n\nVerdict (PASS or FAIL):"
    );
    let out = llm::chat(cfg, sys, &user)
        .await
        .map_err(|e| e.to_string())?;
    Ok(out.content.trim().to_ascii_uppercase().starts_with("PASS"))
}

/// Seeded bootstrap 95% CI on the pass rate (percentile method, 1000 resamples).
/// Seeded so the reported interval is reproducible across runs.
fn bootstrap_ci(results: &[ItemResult], seed: u64) -> (f64, f64) {
    let flags: Vec<u8> = results.iter().map(|r| r.passed as u8).collect();
    if flags.is_empty() {
        return (0.0, 0.0);
    }
    let mut rng = StdRng::seed_from_u64(seed);
    let n = flags.len();
    let mut means: Vec<f64> = (0..1000)
        .map(|_| {
            let sum: u32 = (0..n)
                .map(|_| *flags.choose(&mut rng).unwrap() as u32)
                .sum();
            sum as f64 / n as f64
        })
        .collect();
    means.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (means[24], means[974]) // 2.5th / 97.5th percentile of 1000
}

/// Resolve the dataset file: explicit `--dataset-file`, else download a known
/// dataset to `bench/datasets/<name>/` (cached).
async fn ensure_dataset(args: &BenchArgs) -> Result<PathBuf, String> {
    if let Some(f) = &args.dataset_file {
        return Ok(f.clone());
    }
    match args.dataset.as_str() {
        "frames" => {
            let cache = PathBuf::from("bench/datasets/frames/test.tsv");
            if cache.exists() {
                return Ok(cache);
            }
            let url =
                "https://huggingface.co/datasets/google/frames-benchmark/resolve/main/test.tsv";
            eprintln!("bench: downloading FRAMES → {}", cache.display());
            download(url, &cache).await?;
            Ok(cache)
        }
        other => Err(format!(
            "unknown dataset '{other}'; pass --dataset-file <path> (TSV with Prompt/Answer, or JSONL)"
        )),
    }
}

async fn download(url: &str, dest: &Path) -> Result<(), String> {
    let body = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .map_err(|e| format!("download {url}: {e}"))?
        .error_for_status()
        .map_err(|e| format!("download {url}: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("download body: {e}"))?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    std::fs::write(dest, &body).map_err(|e| format!("write {}: {e}", dest.display()))?;
    Ok(())
}

/// Parse a dataset file: `.tsv` → Prompt/Answer columns; otherwise JSONL with
/// `prompt`/`answer` (or `Prompt`/`Answer`) keys.
fn load_dataset(path: &Path) -> Result<Vec<QaItem>, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    if path.extension().is_some_and(|e| e == "tsv") {
        parse_tsv(&text)
    } else {
        parse_jsonl(&text)
    }
}

// ponytail: naive TSV (split on \n then \t) — correct for FRAMES, whose rows
// are single-line and whose fields hold no tabs/newlines. Swap in a quoted-field
// CSV reader only if a future dataset embeds tabs or newlines in a field.
fn parse_tsv(text: &str) -> Result<Vec<QaItem>, String> {
    let mut lines = text.lines();
    let header = lines.next().ok_or("empty TSV")?;
    let cols: Vec<&str> = header.split('\t').collect();
    let pi = cols
        .iter()
        .position(|c| c.eq_ignore_ascii_case("prompt"))
        .ok_or("TSV missing 'Prompt' column")?;
    let ai = cols
        .iter()
        .position(|c| c.eq_ignore_ascii_case("answer"))
        .ok_or("TSV missing 'Answer' column")?;
    let mut items = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if let (Some(p), Some(a)) = (f.get(pi), f.get(ai))
            && !p.trim().is_empty()
        {
            items.push(QaItem {
                prompt: p.trim().to_string(),
                answer: a.trim().to_string(),
            });
        }
    }
    Ok(items)
}

fn parse_jsonl(text: &str) -> Result<Vec<QaItem>, String> {
    #[derive(Deserialize)]
    struct Row {
        #[serde(alias = "Prompt")]
        prompt: Option<String>,
        #[serde(alias = "Answer")]
        answer: Option<String>,
    }
    let mut items = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row: Row = serde_json::from_str(line).map_err(|e| format!("line {}: {e}", i + 1))?;
        if let (Some(p), Some(a)) = (row.prompt, row.answer)
            && !p.trim().is_empty()
        {
            items.push(QaItem {
                prompt: p,
                answer: a,
            });
        }
    }
    Ok(items)
}

fn write_snapshot(run_dir: &Path, report: &Report, results: &[ItemResult]) -> Result<(), String> {
    std::fs::create_dir_all(run_dir).map_err(|e| format!("mkdir {}: {e}", run_dir.display()))?;

    let mut jsonl = String::new();
    for r in results {
        jsonl.push_str(&serde_json::to_string(r).map_err(|e| e.to_string())?);
        jsonl.push('\n');
    }
    std::fs::write(run_dir.join("frames_results.jsonl"), jsonl).map_err(|e| e.to_string())?;
    std::fs::write(
        run_dir.join("report.json"),
        serde_json::to_string_pretty(report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(run_dir.join("report.md"), report_md(report)).map_err(|e| e.to_string())?;
    Ok(())
}

fn report_md(r: &Report) -> String {
    format!(
        "# crw bench — {dataset}\n\n\
         - provider: `{provider}` @ `{server}`\n\
         - judge: `{judge}`\n\
         - questions: {n}\n\
         - **score: {score:.1}%** ({passed}/{n})\n\
         - 95% CI (bootstrap, seed {seed}): {lo:.1}–{hi:.1}%\n\
         - timestamp (unix): {ts}\n",
        dataset = r.dataset,
        provider = r.provider,
        server = r.server,
        judge = r.judge_model,
        n = r.n,
        score = r.score * 100.0,
        passed = r.passed,
        seed = r.seed,
        lo = r.ci_low * 100.0,
        hi = r.ci_high * 100.0,
        ts = r.timestamp_unix,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tsv_picks_prompt_and_answer_by_header() {
        let tsv = "Prompt\tAnswer\twiki_links\n\
                   What is 2+2?\t4\thttp://x\n\
                   \t\t\n\
                   Capital of France?\tParis\thttp://y\n";
        let items = parse_tsv(tsv).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].prompt, "What is 2+2?");
        assert_eq!(items[0].answer, "4");
        assert_eq!(items[1].answer, "Paris");
    }

    #[test]
    fn parse_jsonl_accepts_both_casings() {
        let jsonl =
            "{\"prompt\":\"q1\",\"answer\":\"a1\"}\n{\"Prompt\":\"q2\",\"Answer\":\"a2\"}\n";
        let items = parse_jsonl(jsonl).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].prompt, "q2");
        assert_eq!(items[1].answer, "a2");
    }

    #[test]
    fn bootstrap_ci_brackets_the_point_estimate() {
        let mk = |pass: bool| ItemResult {
            prompt: String::new(),
            truth: String::new(),
            prediction: String::new(),
            passed: pass,
            error: None,
        };
        // 70/100 pass → score 0.70; CI should bracket it and stay in [0,1].
        let results: Vec<ItemResult> = (0..100).map(|i| mk(i < 70)).collect();
        let (lo, hi) = bootstrap_ci(&results, 42);
        assert!(lo <= 0.70 && 0.70 <= hi, "CI [{lo},{hi}] must bracket 0.70");
        assert!(lo >= 0.0 && hi <= 1.0);
        // Deterministic under a fixed seed.
        assert_eq!((lo, hi), bootstrap_ci(&results, 42));
    }
}
