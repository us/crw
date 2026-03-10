use crw_core::types::FilterMode;
use std::collections::HashMap;

/// Filter and rank chunks by relevance to a query.
pub fn filter_chunks(
    chunks: &[String],
    query: &str,
    mode: &FilterMode,
    top_k: usize,
) -> Vec<String> {
    if chunks.is_empty() || query.trim().is_empty() {
        return chunks.to_vec();
    }
    let k = top_k.max(1).min(chunks.len());
    match mode {
        FilterMode::Bm25 => filter_bm25(chunks, query, k),
        FilterMode::Cosine => filter_cosine(chunks, query, k),
    }
}

// ── Tokenization ──────────────────────────────────────────────────────────────

fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 1)
        .map(|t| t.to_string())
        .collect()
}

// ── BM25 ──────────────────────────────────────────────────────────────────────

const K1: f64 = 1.2;
const B: f64 = 0.75;

fn filter_bm25(chunks: &[String], query: &str, top_k: usize) -> Vec<String> {
    let query_terms = tokenize(query);
    let tokenized: Vec<Vec<String>> = chunks.iter().map(|c| tokenize(c)).collect();

    let n = chunks.len() as f64;
    let avgdl = tokenized.iter().map(|t| t.len()).sum::<usize>() as f64 / n;

    // Document frequency: how many chunks contain each term.
    let mut df: HashMap<&str, usize> = HashMap::new();
    for doc in &tokenized {
        let mut seen: HashMap<&str, bool> = HashMap::new();
        for term in doc {
            if seen.insert(term.as_str(), true).is_none() {
                *df.entry(term.as_str()).or_insert(0) += 1;
            }
        }
    }

    let mut scored: Vec<(usize, f64)> = tokenized
        .iter()
        .enumerate()
        .map(|(i, doc)| {
            let dl = doc.len() as f64;
            let mut tf_map: HashMap<&str, usize> = HashMap::new();
            for term in doc {
                *tf_map.entry(term.as_str()).or_insert(0) += 1;
            }

            let score = query_terms
                .iter()
                .map(|term| {
                    let tf = *tf_map.get(term.as_str()).unwrap_or(&0) as f64;
                    let df_t = *df.get(term.as_str()).unwrap_or(&0) as f64;
                    let idf = ((n - df_t + 0.5) / (df_t + 0.5) + 1.0).ln();
                    let tf_norm = (tf * (K1 + 1.0)) / (tf + K1 * (1.0 - B + B * dl / avgdl));
                    idf * tf_norm
                })
                .sum::<f64>();

            (i, score)
        })
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(top_k);
    scored.into_iter().map(|(i, _)| chunks[i].clone()).collect()
}

// ── Cosine Similarity (TF-IDF) ───────────────────────────────────────────────

fn filter_cosine(chunks: &[String], query: &str, top_k: usize) -> Vec<String> {
    let all_docs: Vec<Vec<String>> = std::iter::once(query.to_string())
        .chain(chunks.iter().cloned())
        .map(|s| tokenize(&s))
        .collect();

    let query_tokens = &all_docs[0];
    let chunk_tokens = &all_docs[1..];

    // Collect all unique terms.
    let mut vocab: Vec<String> = all_docs.iter().flatten().cloned().collect();
    vocab.sort();
    vocab.dedup();

    let n_docs = (1 + chunks.len()) as f64; // query + chunks

    // IDF for each term.
    let idf: Vec<f64> = vocab
        .iter()
        .map(|term| {
            let df = all_docs.iter().filter(|doc| doc.contains(term)).count() as f64;
            ((n_docs - df + 0.5) / (df + 0.5) + 1.0).ln()
        })
        .collect();

    // TF-IDF vector for a token list.
    let tfidf = |tokens: &[String]| -> Vec<f64> {
        let len = tokens.len().max(1) as f64;
        vocab
            .iter()
            .enumerate()
            .map(|(i, term)| {
                let tf = tokens.iter().filter(|t| *t == term).count() as f64 / len;
                tf * idf[i]
            })
            .collect()
    };

    let q_vec = tfidf(query_tokens);
    let q_norm = norm(&q_vec);

    let mut scored: Vec<(usize, f64)> = chunk_tokens
        .iter()
        .enumerate()
        .map(|(i, tokens)| {
            let d_vec = tfidf(tokens);
            let d_norm = norm(&d_vec);
            let sim = if q_norm > 0.0 && d_norm > 0.0 {
                dot(&q_vec, &d_vec) / (q_norm * d_norm)
            } else {
                0.0
            };
            (i, sim)
        })
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(top_k);
    scored.into_iter().map(|(i, _)| chunks[i].clone()).collect()
}

fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

fn norm(v: &[f64]) -> f64 {
    v.iter().map(|x| x * x).sum::<f64>().sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_chunks() -> Vec<String> {
        vec![
            "The quick brown fox jumps over the lazy dog.".into(),
            "Machine learning is a subset of artificial intelligence.".into(),
            "Rust is a systems programming language focused on safety.".into(),
            "Natural language processing enables computers to understand text.".into(),
        ]
    }

    #[test]
    fn bm25_returns_top_k() {
        let chunks = sample_chunks();
        let result = filter_chunks(&chunks, "machine learning AI", &FilterMode::Bm25, 2);
        assert_eq!(result.len(), 2);
        // The ML chunk should be ranked high (case-insensitive check)
        assert!(
            result
                .iter()
                .any(|c| c.to_lowercase().contains("machine learning"))
        );
    }

    #[test]
    fn cosine_returns_top_k() {
        let chunks = sample_chunks();
        let result = filter_chunks(&chunks, "programming language Rust", &FilterMode::Cosine, 2);
        assert_eq!(result.len(), 2);
        assert!(
            result
                .iter()
                .any(|c| c.contains("Rust") || c.contains("language"))
        );
    }

    #[test]
    fn empty_query_returns_all() {
        let chunks = sample_chunks();
        let result = filter_chunks(&chunks, "", &FilterMode::Bm25, 2);
        assert_eq!(result.len(), chunks.len());
    }

    #[test]
    fn top_k_capped_at_chunk_count() {
        let chunks = vec!["a".into(), "b".into()];
        let result = filter_chunks(&chunks, "a", &FilterMode::Bm25, 100);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn ranking_order_is_preserved() {
        let chunks = vec![
            "irrelevant background".to_string(),
            "rust programming language ownership borrow checker".to_string(),
            "rust".to_string(),
        ];

        let result = filter_chunks(&chunks, "rust programming language", &FilterMode::Bm25, 2);
        assert_eq!(result[0], chunks[1]);
    }

    #[test]
    fn bm25_and_cosine_can_diverge() {
        let chunks = vec![
            "token token token token".to_string(),
            "token related semantic context".to_string(),
            "unrelated content".to_string(),
        ];

        let bm25 = filter_chunks(&chunks, "token semantic", &FilterMode::Bm25, 2);
        let cosine = filter_chunks(&chunks, "token semantic", &FilterMode::Cosine, 2);
        assert_ne!(bm25, cosine);
    }
}
