//! Transform a [`SearxngResponse`] into the public flat / grouped result
//! shapes. Direct port of `crw-saas/src/lib/search-transform.ts`.

use std::collections::HashSet;

use crw_core::types::{GroupedSearchData, ImageResult, SearchResult, SearchSource};

use crate::client::{SearxngResponse, SearxngResult};

/// Hard cap on per-source upstream rows we sort/dedupe. SearXNG with all
/// default engines tops out a couple of hundred rows per source bucket;
/// setting 500 leaves comfortable headroom while preventing a misbehaving
/// engine from turning a single search into a CPU/memory amplifier.
const MAX_UPSTREAM_ROWS: usize = 500;

fn score_or_zero(r: &SearxngResult) -> f64 {
    r.score.unwrap_or(0.0)
}

/// Predicate: row carries the load-bearing identity fields (`url`,
/// `title`, `engine`). Real upstreams sometimes emit partial rows — e.g.
/// when an engine times out mid-page — and one bad row used to fail the
/// whole search. We silently skip them and continue.
///
/// Returning a predicate (not a filtered `Vec`) lets each caller chain
/// `.filter(is_well_formed).take(MAX_UPSTREAM_ROWS).cloned()` so we never
/// clone rows that will be discarded. Callers cap *after* filtering by
/// source so a hot bucket (e.g. 600 general results) can't starve a cold
/// one (e.g. 5 news results).
fn is_well_formed(r: &SearxngResult) -> bool {
    r.url.as_deref().is_some_and(|s| !s.is_empty())
        && r.title.as_deref().is_some_and(|s| !s.is_empty())
        && r.engine.as_deref().is_some_and(|s| !s.is_empty())
}

fn url_of(r: &SearxngResult) -> &str {
    r.url.as_deref().unwrap_or("")
}

fn title_of(r: &SearxngResult) -> String {
    r.title.clone().unwrap_or_default()
}

/// Stable-sorted by descending `score` (missing scores treated as 0).
fn sort_by_score(items: &mut [SearxngResult]) {
    items.sort_by(|a, b| {
        score_or_zero(b)
            .partial_cmp(&score_or_zero(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

fn dedupe_by_url(items: Vec<SearxngResult>) -> Vec<SearxngResult> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let key = url_of(&item).to_string();
        if seen.insert(key) {
            out.push(item);
        }
    }
    out
}

fn to_search_result(r: &SearxngResult, position: u32) -> SearchResult {
    SearchResult {
        url: url_of(r).to_string(),
        title: title_of(r),
        description: r.content.clone().unwrap_or_default(),
        position,
        score: r.score,
        published_date: r.published_date.clone(),
        category: r.category.clone(),
        markdown: None,
        html: None,
        raw_html: None,
        links: None,
        metadata: None,
        summary: None,
    }
}

fn to_image_result(r: &SearxngResult, position: u32) -> ImageResult {
    ImageResult {
        url: url_of(r).to_string(),
        title: title_of(r),
        description: r.content.clone().unwrap_or_default(),
        image_url: r.img_src.clone().unwrap_or_else(|| url_of(r).to_string()),
        position,
        thumbnail_url: r.thumbnail_src.clone(),
        image_format: r.img_format.clone(),
        resolution: r.resolution.clone(),
    }
}

/// Flat output: dedupe by URL, sort by score, slice to `limit`.
///
/// Note: SaaS sorts then dedupes, so a higher-scored duplicate wins. We
/// preserve that order — see `crw-saas/src/lib/search-transform.ts:73`.
pub fn transform_flat(response: &SearxngResponse, limit: u32) -> Vec<SearchResult> {
    // Drop malformed rows, then cap the working set at `MAX_UPSTREAM_ROWS`
    // before clone+sort. A misbehaving SearXNG instance (or a query that
    // scoops thousands of rows) would otherwise amplify CPU/memory on every
    // request.
    let mut results: Vec<SearxngResult> = response
        .results
        .iter()
        .filter(|r| is_well_formed(r))
        .take(MAX_UPSTREAM_ROWS)
        .cloned()
        .collect();
    sort_by_score(&mut results);
    let deduped = dedupe_by_url(results);
    deduped
        .into_iter()
        .take(limit as usize)
        .enumerate()
        .map(|(i, r)| to_search_result(&r, (i + 1) as u32))
        .collect()
}

/// Grouped output: filter by `sources`, then per-bucket sort/dedupe/slice.
/// Limit applies **per source**, not in total — matches SaaS semantics.
pub fn transform_grouped(
    response: &SearxngResponse,
    sources: &[SearchSource],
    limit: u32,
) -> GroupedSearchData {
    let mut data = GroupedSearchData::default();
    let cap = limit as usize;

    // Per-source filter+cap on the raw response — `is_well_formed` is a
    // predicate so we never clone rows that will be discarded. Each source
    // gets its own `MAX_UPSTREAM_ROWS` budget for sort/dedupe so a hot
    // bucket (500 web rows) can't starve cold ones (5 news rows).
    if sources.contains(&SearchSource::Web) {
        let mut sorted: Vec<SearxngResult> = response
            .results
            .iter()
            .filter(|r| is_well_formed(r))
            .filter(|r| {
                let cat = r.category.as_deref();
                cat == Some("general") || (r.img_src.is_none() && cat != Some("news"))
            })
            .take(MAX_UPSTREAM_ROWS)
            .cloned()
            .collect();
        sort_by_score(&mut sorted);
        let deduped = dedupe_by_url(sorted);
        data.web = Some(
            deduped
                .into_iter()
                .take(cap)
                .enumerate()
                .map(|(i, r)| to_search_result(&r, (i + 1) as u32))
                .collect(),
        );
    }

    if sources.contains(&SearchSource::News) {
        let mut sorted: Vec<SearxngResult> = response
            .results
            .iter()
            .filter(|r| is_well_formed(r))
            .filter(|r| r.category.as_deref() == Some("news"))
            .take(MAX_UPSTREAM_ROWS)
            .cloned()
            .collect();
        sort_by_score(&mut sorted);
        let deduped = dedupe_by_url(sorted);
        data.news = Some(
            deduped
                .into_iter()
                .take(cap)
                .enumerate()
                .map(|(i, r)| to_search_result(&r, (i + 1) as u32))
                .collect(),
        );
    }

    if sources.contains(&SearchSource::Images) {
        let mut sorted: Vec<SearxngResult> = response
            .results
            .iter()
            .filter(|r| is_well_formed(r))
            .filter(|r| r.category.as_deref() == Some("images") || r.img_src.is_some())
            .take(MAX_UPSTREAM_ROWS)
            .cloned()
            .collect();
        sort_by_score(&mut sorted);
        let deduped = dedupe_by_url(sorted);
        data.images = Some(
            deduped
                .into_iter()
                .take(cap)
                .enumerate()
                .map(|(i, r)| to_image_result(&r, (i + 1) as u32))
                .collect(),
        );
    }

    data
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(url: &str, score: f64, content: &str) -> SearxngResult {
        SearxngResult {
            url: Some(url.into()),
            title: Some(format!("title-{url}")),
            engine: Some("test".into()),
            content: Some(content.into()),
            score: Some(score),
            category: Some("general".into()),
            template: None,
            published_date: None,
            img_src: None,
            thumbnail_src: None,
            img_format: None,
            resolution: None,
        }
    }

    fn news(url: &str, score: f64) -> SearxngResult {
        SearxngResult {
            url: Some(url.into()),
            title: Some(format!("news-{url}")),
            engine: Some("test".into()),
            content: Some("snippet".into()),
            score: Some(score),
            category: Some("news".into()),
            template: None,
            published_date: Some("2026-05-01T00:00:00Z".into()),
            img_src: None,
            thumbnail_src: None,
            img_format: None,
            resolution: None,
        }
    }

    fn image(url: &str, score: f64, img: &str) -> SearxngResult {
        SearxngResult {
            url: Some(url.into()),
            title: Some(format!("img-{url}")),
            engine: Some("test".into()),
            content: Some(String::new()),
            score: Some(score),
            category: Some("images".into()),
            template: Some("images.html".into()),
            published_date: None,
            img_src: Some(img.into()),
            thumbnail_src: Some(format!("{img}.thumb")),
            img_format: Some("jpeg".into()),
            resolution: Some("1920x1080".into()),
        }
    }

    fn resp(items: Vec<SearxngResult>) -> SearxngResponse {
        SearxngResponse {
            results: items,
            ..SearxngResponse::default()
        }
    }

    #[test]
    fn flat_sorts_by_score_desc() {
        let res = transform_flat(
            &resp(vec![r("a", 0.1, "A"), r("b", 0.9, "B"), r("c", 0.5, "C")]),
            5,
        );
        assert_eq!(
            res.iter().map(|x| x.url.as_str()).collect::<Vec<_>>(),
            vec!["b", "c", "a"]
        );
        assert_eq!(res[0].position, 1);
        assert_eq!(res[1].position, 2);
        assert_eq!(res[2].position, 3);
    }

    #[test]
    fn flat_dedupe_keeps_highest_score() {
        let res = transform_flat(&resp(vec![r("a", 0.1, "low"), r("a", 0.9, "high")]), 5);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].description, "high");
    }

    #[test]
    fn flat_respects_limit() {
        let res = transform_flat(
            &resp(vec![r("a", 0.9, "A"), r("b", 0.8, "B"), r("c", 0.7, "C")]),
            2,
        );
        assert_eq!(res.len(), 2);
    }

    #[test]
    fn flat_missing_score_treated_as_zero() {
        let mut a = r("a", 0.0, "A");
        a.score = None;
        let res = transform_flat(&resp(vec![a, r("b", 0.5, "B")]), 5);
        assert_eq!(res[0].url, "b");
    }

    #[test]
    fn grouped_web_filters_general_and_unknown() {
        let res = transform_grouped(
            &resp(vec![
                r("g", 0.9, ""),
                news("n", 0.8),
                image("i", 0.7, "https://i.img"),
            ]),
            &[SearchSource::Web],
            5,
        );
        let web = res.web.unwrap();
        assert_eq!(
            web.iter().map(|x| x.url.as_str()).collect::<Vec<_>>(),
            vec!["g"]
        );
    }

    #[test]
    fn grouped_news_only_news_category() {
        let res = transform_grouped(
            &resp(vec![r("g", 0.9, ""), news("n1", 0.8), news("n2", 0.6)]),
            &[SearchSource::News],
            5,
        );
        let n = res.news.unwrap();
        assert_eq!(n.len(), 2);
        assert_eq!(n[0].url, "n1");
        assert!(n[0].published_date.is_some());
    }

    #[test]
    fn grouped_images_picks_image_or_img_src() {
        let mut general_with_img = r("g", 0.5, "");
        general_with_img.img_src = Some("https://x.png".into());

        let res = transform_grouped(
            &resp(vec![image("i", 0.9, "https://i.img"), general_with_img]),
            &[SearchSource::Images],
            5,
        );
        let imgs = res.images.unwrap();
        assert_eq!(imgs.len(), 2);
        assert_eq!(imgs[0].url, "i");
        assert_eq!(imgs[0].image_url, "https://i.img");
    }

    #[test]
    fn grouped_image_falls_back_to_url_when_img_src_missing() {
        let mut img = image("i", 0.9, "");
        img.img_src = None; // category=images but no img_src
        let res = transform_grouped(&resp(vec![img]), &[SearchSource::Images], 5);
        let imgs = res.images.unwrap();
        assert_eq!(imgs[0].image_url, "i"); // falls back to url
    }

    #[test]
    fn grouped_limit_applies_per_source() {
        let mut items = vec![];
        for i in 0..5 {
            items.push(r(&format!("g{i}"), 1.0 - i as f64 * 0.1, ""));
            items.push(news(&format!("n{i}"), 1.0 - i as f64 * 0.1));
        }
        let res = transform_grouped(&resp(items), &[SearchSource::Web, SearchSource::News], 2);
        assert_eq!(res.web.unwrap().len(), 2);
        assert_eq!(res.news.unwrap().len(), 2);
    }

    #[test]
    fn grouped_hot_bucket_does_not_starve_cold_buckets() {
        // Regression for codex review iteration 2: the well-formed cap used
        // to be applied globally before per-source filtering, so 600 web
        // rows could push all the news rows out of the working set. Now the
        // cap is per-source — both buckets must populate.
        let mut items = Vec::new();
        for i in 0..600 {
            items.push(r(&format!("g{i}"), 1.0 - (i as f64 / 1000.0), ""));
        }
        for i in 0..3 {
            items.push(news(&format!("n{i}"), 0.5));
        }
        let res = transform_grouped(&resp(items), &[SearchSource::Web, SearchSource::News], 10);
        assert_eq!(res.web.unwrap().len(), 10);
        assert_eq!(
            res.news.unwrap().len(),
            3,
            "cold news bucket must survive a hot web bucket"
        );
    }

    #[test]
    fn malformed_rows_are_dropped_silently() {
        // Mix of well-formed and malformed rows: missing url, missing title,
        // empty engine. Only the well-formed row should survive.
        let mut bad_url = r("ok", 0.9, "ok-snippet");
        bad_url.url = None;
        let mut empty_title = r("x", 0.5, "x");
        empty_title.title = Some(String::new());
        let mut no_engine = r("y", 0.4, "y");
        no_engine.engine = None;
        let good = r("z", 0.3, "z");
        let res = transform_flat(&resp(vec![bad_url, empty_title, no_engine, good]), 10);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].url, "z");
    }

    #[test]
    fn grouped_unrequested_source_omitted() {
        let res = transform_grouped(&resp(vec![r("g", 0.9, "")]), &[SearchSource::Web], 5);
        assert!(res.web.is_some());
        assert!(res.news.is_none());
        assert!(res.images.is_none());
    }
}
