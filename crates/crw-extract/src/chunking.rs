use crw_core::types::ChunkStrategy;
use regex::Regex;

/// Chunk text according to the given strategy.
pub fn chunk_text(text: &str, strategy: &ChunkStrategy) -> Vec<String> {
    match strategy {
        ChunkStrategy::Sentence { max_chars } => chunk_by_sentence(text, *max_chars),
        ChunkStrategy::Regex { pattern } => chunk_by_regex(text, pattern),
        ChunkStrategy::Topic => chunk_by_topic(text),
    }
}

/// Split on sentence boundaries (.!?) then merge chunks that are too short.
fn chunk_by_sentence(text: &str, max_chars: Option<usize>) -> Vec<String> {
    let max = max_chars.unwrap_or(1000);
    let min_merge = max / 4; // Merge if a chunk is shorter than 25% of max.

    // Split on sentence-ending punctuation followed by whitespace or end.
    let re = Regex::new(r"(?<=[.!?])\s+").unwrap_or_else(|_| Regex::new(r"\s{2,}").unwrap());
    let raw: Vec<&str> = re.split(text).collect();

    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();

    for sentence in raw {
        let sentence = sentence.trim();
        if sentence.is_empty() {
            continue;
        }

        if current.is_empty() {
            current.push_str(sentence);
        } else if current.len() + sentence.len() + 1 <= max {
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
        let chunks = chunk_text(text, &ChunkStrategy::Topic);
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
            },
        );
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], text);
    }
}
