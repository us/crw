use crw_core::types::OutputFormat;
use crw_extract::ExtractOptions;

fn main() {
    let html = r##"<html><head>
        <title>New extended temperature range for Compute Module 4 - Raspberry Pi</title>
        <meta property="og:title" content="New extended temperature range for Compute Module 4 - Raspberry Pi">
    </head><body>
        <nav><h1>News</h1><a href="/news">All news</a></nav>
        <article><p>While the Raspberry Pi project has its origins in education, the majority of Raspberry Pi computers we make today are destined for industrial and embedded applications. Compute Module 4 has been used by thousands of customers in challenging environments.</p></article>
    </body></html>"##;
    let data = crw_extract::extract(ExtractOptions {
        raw_html: html,
        source_url: "https://www.raspberrypi.com/news/x/",
        status_code: 200,
        rendered_with: Some("http".into()),
        elapsed_ms: 0,
        render_decision: None,
        credit_cost: 0,
        warnings: Vec::new(),
        formats: &[OutputFormat::Markdown],
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
    })
    .unwrap();
    let md = data.markdown.unwrap();
    println!("og_title: {:?}", data.metadata.og_title);
    println!("md_len: {}", md.len());
    println!("first 200:\n{}", &md[..md.len().min(200)]);
    println!();
    println!(
        "contains core: {}",
        md.contains("New extended temperature range for Compute Module 4")
    );
    println!(
        "starts with prepended title H1: {}",
        md.starts_with("# New extended temperature range")
    );
}
