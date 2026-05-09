//! Data-table vs. layout-table classifier.
//!
//! Returns a numeric score; threshold (default `7.0`, configurable via
//! `ExtractionConfig::data_table_score_threshold`) decides whether a
//! `<table>` should bypass short-text pruning during cleanup.
//!
//! Features mirror the structural hints real data tables carry that
//! layout tables (newsletter shells, ad slots) generally lack.

use scraper::{ElementRef, Selector};

const DATA_TABLE_SCORE_THRESHOLD: f64 = 7.0;

/// Score a `<table>` element by structural data-table indicators.
/// Higher = more likely a real data table. Threshold gate at
/// `DATA_TABLE_SCORE_THRESHOLD` for the bypass-short-prune decision.
pub fn is_data_table(table: ElementRef<'_>) -> f64 {
    let mut score: f64 = 0.0;

    let thead_sel = Selector::parse("thead").unwrap();
    let tbody_sel = Selector::parse("tbody").unwrap();
    let th_sel = Selector::parse("th").unwrap();
    let tr_sel = Selector::parse("tr").unwrap();
    let nested_sel = Selector::parse("table table").unwrap();
    let caption_sel = Selector::parse("caption").unwrap();
    let td_sel = Selector::parse("td").unwrap();

    if table.select(&thead_sel).next().is_some() {
        score += 2.0;
    }
    if table.select(&tbody_sel).next().is_some() {
        score += 1.0;
    }

    let th_count = table.select(&th_sel).count();
    if th_count > 0 {
        score += 2.0;
        if first_row_or_thead_has_th(table) {
            score += 1.0;
        }
    }

    if table.select(&nested_sel).next().is_some() {
        score -= 3.0;
    }

    if let Some(role) = table.value().attr("role")
        && (role.eq_ignore_ascii_case("presentation") || role.eq_ignore_ascii_case("none"))
    {
        score -= 3.0;
    }

    let col_counts: Vec<usize> = table
        .select(&tr_sel)
        .map(|tr| {
            let td_in_row = Selector::parse("td, th").unwrap();
            tr.select(&td_in_row).count()
        })
        .filter(|c| *c > 0)
        .collect();
    if col_counts.len() >= 2 {
        let mean = col_counts.iter().sum::<usize>() as f64 / col_counts.len() as f64;
        let variance: f64 = col_counts
            .iter()
            .map(|c| {
                let d = *c as f64 - mean;
                d * d
            })
            .sum::<f64>()
            / col_counts.len() as f64;
        if variance < 1.0 {
            score += 2.0;
        }
    }

    if table.select(&caption_sel).next().is_some() {
        score += 2.0;
    }
    if table.value().attr("summary").is_some() {
        score += 1.0;
    }

    let total_text_chars: usize = table.text().map(|s| s.chars().count()).sum();
    let tag_count = count_descendant_tags(table);
    let density = total_text_chars as f64 / tag_count.max(1) as f64;
    if density > 20.0 {
        score += 3.0;
    } else if density > 10.0 {
        score += 2.0;
    }

    let data_attr_count = count_data_attrs(table);
    score += 0.5 * data_attr_count as f64;

    let row_count = col_counts.len();
    let avg_cols = if col_counts.is_empty() {
        0.0
    } else {
        col_counts.iter().sum::<usize>() as f64 / col_counts.len() as f64
    };
    if row_count >= 2 && avg_cols >= 2.0 {
        score += 2.0;
    }

    let total_cells: usize = table.select(&td_sel).count();
    let _ = total_cells;

    score
}

/// Convenience: whether a table clears the data-table threshold.
pub fn is_likely_data_table(table: ElementRef<'_>) -> bool {
    is_data_table(table) >= DATA_TABLE_SCORE_THRESHOLD
}

fn first_row_or_thead_has_th(table: ElementRef<'_>) -> bool {
    let thead_sel = Selector::parse("thead").unwrap();
    let th_sel = Selector::parse("th").unwrap();
    let tr_sel = Selector::parse("tr").unwrap();
    if let Some(thead) = table.select(&thead_sel).next()
        && thead.select(&th_sel).next().is_some()
    {
        return true;
    }
    if let Some(first_row) = table.select(&tr_sel).next() {
        return first_row.select(&th_sel).next().is_some();
    }
    false
}

fn count_descendant_tags(el: ElementRef<'_>) -> usize {
    let mut n = 0;
    for desc in el.descendants() {
        if desc.value().is_element() {
            n += 1;
        }
    }
    n
}

fn count_data_attrs(el: ElementRef<'_>) -> usize {
    el.value()
        .attrs()
        .filter(|(k, _)| k.starts_with("data-"))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use scraper::Html;

    fn score_first_table(html: &str) -> f64 {
        let doc = Html::parse_document(html);
        let sel = Selector::parse("table").unwrap();
        let table = doc.select(&sel).next().expect("no table in fixture");
        is_data_table(table)
    }

    #[test]
    fn data_table_with_thead_th_caption_scores_high() {
        let html = r#"<!doctype html><html><body><table>
            <caption>Quarterly results</caption>
            <thead><tr><th>Q1</th><th>Q2</th><th>Q3</th></tr></thead>
            <tbody>
                <tr><td>10</td><td>20</td><td>30</td></tr>
                <tr><td>11</td><td>21</td><td>31</td></tr>
                <tr><td>12</td><td>22</td><td>32</td></tr>
            </tbody>
        </table></body></html>"#;
        let score = score_first_table(html);
        assert!(score >= 7.0, "expected >=7.0 got {score}");
    }

    #[test]
    fn layout_table_with_role_presentation_scores_low() {
        let html = r#"<!doctype html><html><body><table role="presentation">
            <tr><td><img src="logo.png"></td></tr>
            <tr><td>Newsletter content</td></tr>
        </table></body></html>"#;
        let score = score_first_table(html);
        assert!(score < 7.0, "expected <7.0 got {score}");
    }

    #[test]
    fn nested_table_penalised() {
        let html = r#"<!doctype html><html><body><table>
            <tr><td>outer<table><tr><td>inner</td></tr></table></td></tr>
        </table></body></html>"#;
        let score = score_first_table(html);
        assert!(score < 7.0, "expected <7.0 got {score}");
    }
}
