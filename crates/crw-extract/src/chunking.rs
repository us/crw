use crw_core::types::ChunkStrategy;
use regex::Regex;
use std::collections::HashSet;

#[derive(Clone, Copy)]
struct ChunkOptions {
    max_chars: Option<usize>,
    overlap_chars: usize,
    dedupe: bool,
}

/// Chunk text according to the given strategy.
pub fn chunk_text(text: &str, strategy: &ChunkStrategy) -> Vec<String> {
    let (chunks, options) = match strategy {
        ChunkStrategy::Sentence {
            max_chars,
            overlap_chars,
            dedupe,
        } => (
            chunk_by_sentence(text, *max_chars),
            ChunkOptions {
                max_chars: *max_chars,
                overlap_chars: overlap_chars.unwrap_or(0),
                dedupe: dedupe.unwrap_or(false),
            },
        ),
        ChunkStrategy::Regex {
            pattern,
            max_chars,
            overlap_chars,
            dedupe,
        } => (
            chunk_by_regex(text, pattern),
            ChunkOptions {
                max_chars: *max_chars,
                overlap_chars: overlap_chars.unwrap_or(0),
                dedupe: dedupe.unwrap_or(false),
            },
        ),
        ChunkStrategy::Topic {
            max_chars,
            overlap_chars,
            dedupe,
        } => (
            chunk_by_topic(text),
            ChunkOptions {
                max_chars: *max_chars,
                overlap_chars: overlap_chars.unwrap_or(0),
                dedupe: dedupe.unwrap_or(false),
            },
        ),
    };

    post_process_chunks(chunks, options)
}

/// Split on sentence boundaries (.!?) then merge chunks that are too short.
fn chunk_by_sentence(text: &str, max_chars: Option<usize>) -> Vec<String> {
    let max = max_chars.unwrap_or(1000);
    let min_merge = max / 4; // Merge if a chunk is shorter than 25% of max.

    // Split on sentence boundaries: find [.!?] followed by whitespace, keep punctuation
    // with the preceding sentence. Rust regex doesn't support lookbehind.
    let boundary = Regex::new(r"[.!?]+\s+").unwrap();
    let mut raw: Vec<String> = Vec::new();
    let mut last = 0;
    for m in boundary.find_iter(text) {
        // include the punctuation (everything up to the trailing whitespace)
        let end = m.start() + m.as_str().trim_end().len();
        let fragment = text[last..end].trim();
        if !fragment.is_empty() {
            raw.push(fragment.to_string());
        }
        last = m.end();
    }
    if last < text.len() {
        let tail = text[last..].trim();
        if !tail.is_empty() {
            raw.push(tail.to_string());
        }
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();

    for sentence in &raw {
        let sentence = sentence.trim();
        if sentence.is_empty() {
            continue;
        }

        if current.is_empty() {
            current.push_str(sentence);
        } else if current.len() + sentence.len() + 1 < max {
            current.push(' ');
            current.push_str(sentence);
        } else {
            chunks.push(current.trim().to_string());
            current = sentence.to_string();
        }
    }
    if !current.trim().is_empty() {
        chunks.push(current.trim().to_string());
    }

    // Merge very short trailing chunks into the previous one.
    let mut merged: Vec<String> = Vec::new();
    for chunk in chunks {
        if chunk.len() < min_merge && !merged.is_empty() {
            let last = merged.last_mut().unwrap();
            last.push(' ');
            last.push_str(&chunk);
        } else {
            merged.push(chunk);
        }
    }

    merged
}

fn post_process_chunks(chunks: Vec<String>, options: ChunkOptions) -> Vec<String> {
    let mut processed = if let Some(max_chars) = options.max_chars.filter(|max| *max > 0) {
        chunks
            .into_iter()
            .flat_map(|chunk| split_long_chunk(&chunk, max_chars, options.overlap_chars))
            .collect::<Vec<_>>()
    } else {
        chunks
    };

    processed.retain(|chunk| !chunk.trim().is_empty());

    if options.dedupe {
        let mut seen = HashSet::new();
        processed.retain(|chunk| seen.insert(normalize_chunk(chunk)));
    }

    processed
}

fn split_long_chunk(chunk: &str, max_chars: usize, overlap_chars: usize) -> Vec<String> {
    let text = chunk.trim();
    if text.is_empty() || text.len() <= max_chars {
        return if text.is_empty() {
            Vec::new()
        } else {
            vec![text.to_string()]
        };
    }

    let mut result = Vec::new();
    let mut start = 0;
    let overlap_chars = overlap_chars.min(max_chars.saturating_sub(1));

    while start < text.len() {
        while start < text.len() && !text.is_char_boundary(start) {
            start += 1;
        }

        let remaining = &text[start..];
        if remaining.len() <= max_chars {
            result.push(remaining.trim().to_string());
            break;
        }

        let mut end = start + max_chars;
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }

        if let Some(relative) = text[start..end].rfind(|c: char| c.is_whitespace())
            && relative > max_chars / 2
        {
            end = start + relative;
        }

        let segment = text[start..end].trim();
        if !segment.is_empty() {
            result.push(segment.to_string());
        }

        if end >= text.len() {
            break;
        }

        let step = end
            .saturating_sub(start)
            .saturating_sub(overlap_chars)
            .max(1);
        start += step;
    }

    result
}

fn normalize_chunk(chunk: &str) -> String {
    chunk
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Split using a regex pattern as separator.
fn chunk_by_regex(text: &str, pattern: &str) -> Vec<String> {
    let re = match Regex::new(pattern) {
        Ok(r) => r,
        Err(_) => {
            tracing::warn!("Invalid chunk regex pattern: {pattern}");
            return vec![text.to_string()];
        }
    };
    re.split(text)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Split on markdown headings (lines starting with #).
fn chunk_by_topic(text: &str) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();

    for line in text.lines() {
        if line.starts_with('#') && !current.trim().is_empty() {
            chunks.push(current.trim().to_string());
            current = String::new();
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.trim().is_empty() {
        chunks.push(current.trim().to_string());
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentence_chunks_basic() {
        let text = "Hello world. This is sentence two. And sentence three.";
        let chunks = chunk_text(
            text,
            &ChunkStrategy::Sentence {
                max_chars: Some(30),
                overlap_chars: None,
                dedupe: None,
            },
        );
        assert!(!chunks.is_empty());
        // Each chunk should not exceed max_chars significantly
        for chunk in &chunks {
            assert!(chunk.len() <= 60, "Chunk too long: {chunk}");
        }
    }

    #[test]
    fn topic_chunks_on_headings() {
        let text =
            "# Title\nContent under title.\n## Section\nSection content.\n### Sub\nSub content.";
        let chunks = chunk_text(
            text,
            &ChunkStrategy::Topic {
                max_chars: None,
                overlap_chars: None,
                dedupe: None,
            },
        );
        assert_eq!(chunks.len(), 3);
        assert!(chunks[0].starts_with("# Title"));
        assert!(chunks[1].starts_with("## Section"));
    }

    #[test]
    fn regex_chunk_on_double_newline() {
        let text = "Para one.\n\nPara two.\n\nPara three.";
        let chunks = chunk_text(
            text,
            &ChunkStrategy::Regex {
                pattern: r"\n\n".to_string(),
                max_chars: None,
                overlap_chars: None,
                dedupe: None,
            },
        );
        assert_eq!(chunks.len(), 3);
    }

    #[test]
    fn regex_invalid_pattern_returns_whole_text() {
        let text = "some text";
        let chunks = chunk_text(
            text,
            &ChunkStrategy::Regex {
                pattern: "[invalid".to_string(),
                max_chars: None,
                overlap_chars: None,
                dedupe: None,
            },
        );
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], text);
    }

    #[test]
    fn overlap_and_dedupe_are_applied() {
        let text = "alpha beta gamma delta epsilon zeta eta theta";
        let chunks = chunk_text(
            text,
            &ChunkStrategy::Regex {
                pattern: r"\n\n".to_string(),
                max_chars: Some(16),
                overlap_chars: Some(5),
                dedupe: Some(true),
            },
        );

        assert!(chunks.len() >= 2);
        assert!(chunks.iter().all(|chunk| chunk.len() <= 16));
    }
}
