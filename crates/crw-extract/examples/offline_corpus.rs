//! Run the real extraction path over a frozen corpus of raw HTML, offline.
//!
//! Lets an extractor change be scored against the benchmark set without
//! refetching a thousand URLs for every iteration.
//!
//! in:  jsonl `{"url": ..., "raw_html": ...}`
//! out: jsonl `{"url": ..., "markdown": ..., "chars": n, "micros": n}`
//! usage: cargo run --release --example offline_corpus -- <in.jsonl> <out.jsonl>

use crw_core::types::OutputFormat;
use crw_extract::ExtractOptions;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rd = BufReader::new(File::open(&args[1]).expect("open input"));
    let mut wr = BufWriter::new(File::create(&args[2]).expect("create output"));
    let formats = [OutputFormat::Markdown];

    let (mut n, mut empty) = (0usize, 0usize);
    let t0 = Instant::now();

    for line in rd.lines() {
        let line = line.expect("read line");
        let v: serde_json::Value = serde_json::from_str(&line).expect("parse row");
        let url = v["url"].as_str().unwrap_or("");
        let html = v["raw_html"].as_str().unwrap_or("");
        n += 1;

        let t = Instant::now();
        let md = if html.is_empty() {
            String::new()
        } else {
            crw_extract::extract(ExtractOptions {
                raw_html: html,
                source_url: url,
                status_code: 200,
                rendered_with: Some("http".into()),
                elapsed_ms: 0,
                render_decision: None,
                credit_cost: 0,
                warnings: Vec::new(),
                formats: &formats,
                only_main_content: true,
                include_tags: &[],
                exclude_tags: &[],
                css_selector: None,
                xpath: None,
                chunk_strategy: None,
                query: None,
                filter_mode: None,
                top_k: None,
                domain_selectors: None,
                captured_responses: &[],
                llm_fallback: None,
                debug: false,
                debug_sink: None,
                normalize_tables: false,
            })
            .ok()
            .and_then(|d| d.markdown)
            .unwrap_or_default()
        };
        let micros = t.elapsed().as_micros();
        if md.trim().is_empty() {
            empty += 1;
        }

        writeln!(
            wr,
            "{}",
            serde_json::json!({
                "url": url,
                "markdown": md,
                "chars": md.len(),
                "micros": micros,
            })
        )
        .expect("write row");
    }

    eprintln!(
        "rows {n} | empty {empty} | {:.1}s",
        t0.elapsed().as_secs_f64()
    );
}
