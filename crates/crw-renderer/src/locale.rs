//! Country to locale alignment for the residential proxy exit.
//!
//! When a request pins a proxy exit country (`ScrapeRequest.country` ->
//! [`crate::REQUEST_COUNTRY`]), the browser must not keep advertising a US
//! locale and a UTC clock: an IP/locale mismatch is a stronger bot signal than
//! a flagged IP on its own. This module maps a country code to the
//! `Accept-Language` and IANA timezone a real visitor from there would present.
//!
//! Purely additive: no country (or an unknown one) yields `None` and every
//! caller keeps its previous behaviour byte for byte.

/// The locale a visitor from one country presents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Locale {
    /// Full `Accept-Language` header value.
    pub accept_language: &'static str,
    /// IANA timezone id for `Emulation.setTimezoneOverride`.
    pub timezone: &'static str,
}

impl Locale {
    /// The primary language tag, e.g. `de-DE`, for `Emulation.setLocaleOverride`.
    pub fn primary_tag(&self) -> &'static str {
        self.accept_language
            .split(',')
            .next()
            .unwrap_or(self.accept_language)
    }

    /// The language tags without quality weights: `de-DE,de;q=0.9,en;q=0.8`
    /// becomes `de-DE`, `de`, `en`.
    fn tags(&self) -> impl Iterator<Item = &'static str> {
        self.accept_language.split(',').filter_map(|part| {
            let tag = part.split(';').next().unwrap_or(part).trim();
            (!tag.is_empty()).then_some(tag)
        })
    }

    /// The value for CDP's `Network.setUserAgentOverride.acceptLanguage`:
    /// a plain comma-separated tag list, `de-DE,de,en`. Chromium adds its own
    /// quality weights to that list, so handing it the weighted header would
    /// produce `de-DE,de;q=0.9;q=0.9,...`, a malformed and fingerprintable
    /// header.
    pub fn cdp_accept_language(&self) -> String {
        self.tags().collect::<Vec<_>>().join(",")
    }

    /// The value for the stealth script's `navigator.languages`, as a JS array
    /// literal: `de-DE,de;q=0.9,en;q=0.8` becomes `['de-DE', 'de', 'en']`.
    pub fn js_languages(&self) -> String {
        let tags: Vec<String> = self.tags().map(|tag| format!("'{tag}'")).collect();
        format!("[{}]", tags.join(", "))
    }
}

/// Country code (lowercase alpha-2) to locale.
///
/// One language and one timezone per country: the capital's zone, or the most
/// populous zone for a country that spans several. The row set is the
/// countries we expect to see requested, not a provider's exit list.
///
/// ponytail: a flat table and a linear scan. The known ceilings are one zone
/// per country (wrong for a US west-coast or a Siberian exit) and one language
/// per country (wrong for Quebec, Catalonia, Wallonia). Upgrade path if that
/// ever matters: key on the proxy's reported city rather than its country, and
/// return a list of candidate locales to pick from.
const LOCALES: &[(&str, Locale)] = &[
    (
        "ar",
        loc("es-AR,es;q=0.9,en;q=0.8", "America/Argentina/Buenos_Aires"),
    ),
    ("at", loc("de-AT,de;q=0.9,en;q=0.8", "Europe/Vienna")),
    ("au", loc("en-AU,en;q=0.9", "Australia/Sydney")),
    ("be", loc("nl-BE,nl;q=0.9,en;q=0.8", "Europe/Brussels")),
    ("br", loc("pt-BR,pt;q=0.9,en;q=0.8", "America/Sao_Paulo")),
    ("ca", loc("en-CA,en;q=0.9", "America/Toronto")),
    ("ch", loc("de-CH,de;q=0.9,en;q=0.8", "Europe/Zurich")),
    ("cn", loc("zh-CN,zh;q=0.9,en;q=0.8", "Asia/Shanghai")),
    ("cz", loc("cs-CZ,cs;q=0.9,en;q=0.8", "Europe/Prague")),
    ("de", loc("de-DE,de;q=0.9,en;q=0.8", "Europe/Berlin")),
    ("dk", loc("da-DK,da;q=0.9,en;q=0.8", "Europe/Copenhagen")),
    ("es", loc("es-ES,es;q=0.9,en;q=0.8", "Europe/Madrid")),
    ("fi", loc("fi-FI,fi;q=0.9,en;q=0.8", "Europe/Helsinki")),
    ("fr", loc("fr-FR,fr;q=0.9,en;q=0.8", "Europe/Paris")),
    ("gb", loc("en-GB,en;q=0.9", "Europe/London")),
    ("ie", loc("en-IE,en;q=0.9", "Europe/Dublin")),
    ("in", loc("en-IN,en;q=0.9", "Asia/Kolkata")),
    ("it", loc("it-IT,it;q=0.9,en;q=0.8", "Europe/Rome")),
    ("jp", loc("ja-JP,ja;q=0.9,en;q=0.8", "Asia/Tokyo")),
    ("kr", loc("ko-KR,ko;q=0.9,en;q=0.8", "Asia/Seoul")),
    ("mx", loc("es-MX,es;q=0.9,en;q=0.8", "America/Mexico_City")),
    ("nl", loc("nl-NL,nl;q=0.9,en;q=0.8", "Europe/Amsterdam")),
    ("no", loc("nb-NO,nb;q=0.9,en;q=0.8", "Europe/Oslo")),
    ("nz", loc("en-NZ,en;q=0.9", "Pacific/Auckland")),
    ("pl", loc("pl-PL,pl;q=0.9,en;q=0.8", "Europe/Warsaw")),
    ("pt", loc("pt-PT,pt;q=0.9,en;q=0.8", "Europe/Lisbon")),
    ("ru", loc("ru-RU,ru;q=0.9,en;q=0.8", "Europe/Moscow")),
    ("se", loc("sv-SE,sv;q=0.9,en;q=0.8", "Europe/Stockholm")),
    ("tr", loc("tr-TR,tr;q=0.9,en;q=0.8", "Europe/Istanbul")),
    ("ua", loc("uk-UA,uk;q=0.9,en;q=0.8", "Europe/Kyiv")),
    // Not an ISO code, but the proxy credential passes it through untouched
    // and providers commonly treat it as an alias of GB.
    ("uk", loc("en-GB,en;q=0.9", "Europe/London")),
    ("us", loc("en-US,en;q=0.9", "America/New_York")),
];

const fn loc(accept_language: &'static str, timezone: &'static str) -> Locale {
    Locale {
        accept_language,
        timezone,
    }
}

/// Look up the locale for an ISO 3166-1 alpha-2 country code.
///
/// Accepts any case and surrounding whitespace, the same normalization the
/// proxy-credential composition applies. Returns `None` for a malformed code
/// or a country we have no row for, which keeps every caller on its default.
pub fn locale_for_country(country: &str) -> Option<Locale> {
    let cc = country.trim();
    if cc.len() != 2 || !cc.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    LOCALES
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(cc))
        .map(|(_, locale)| *locale)
}

/// The locale for the country pinned on the current request, if any.
///
/// ponytail: reads only [`crate::REQUEST_COUNTRY`], not
/// `renderer.proxy_default_country`. The HTTP fetcher has no handle on the
/// renderer config and threading one through its three constructors buys
/// nothing today, since no deployment sets a default country. One asymmetry
/// follows: the CDP tier's country-fallback retry re-scopes `REQUEST_COUNTRY`
/// to that default before it re-runs, so only the retry attempt carries the
/// default country's locale. Upgrade path: resolve request country -> default
/// once where `REQUEST_COUNTRY` is scoped, so every reader sees the same value.
pub fn request_locale() -> Option<Locale> {
    crate::REQUEST_COUNTRY
        .try_with(|c| c.clone())
        .ok()
        .flatten()
        .as_deref()
        .and_then(locale_for_country)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_table_is_well_formed() {
        let mut seen: Vec<&str> = Vec::new();
        for (cc, locale) in LOCALES {
            assert_eq!(cc.len(), 2, "{cc}: key must be alpha-2");
            assert!(
                cc.chars().all(|c| c.is_ascii_lowercase()),
                "{cc}: key must be lowercase"
            );
            assert!(!seen.contains(cc), "{cc}: duplicate row");
            seen.push(cc);

            // First tag is a region-qualified language, e.g. "de-DE".
            let primary = locale.primary_tag();
            let (lang, region) = primary
                .split_once('-')
                .unwrap_or_else(|| panic!("{primary}: needs a region"));
            assert_eq!(lang.len(), 2, "{primary}: language subtag");
            // "uk" is the one alias row: it carries GB's locale.
            let expected_region = if *cc == "uk" { "gb" } else { cc };
            assert!(
                region.eq_ignore_ascii_case(expected_region),
                "{primary}: region must match the row key {cc}"
            );

            // Timezone is an IANA "Area/Location" id.
            assert!(
                locale.timezone.contains('/') && !locale.timezone.contains(' '),
                "{}: not an IANA zone id",
                locale.timezone
            );
        }
        assert!(seen.len() >= 30, "table shrank unexpectedly");
    }

    #[test]
    fn locale_lookup_normalizes_input() {
        let berlin = "Europe/Berlin";
        assert_eq!(locale_for_country("de").unwrap().timezone, berlin);
        assert_eq!(locale_for_country("DE").unwrap().timezone, berlin);
        assert_eq!(locale_for_country("  de  ").unwrap().timezone, berlin);
        // Malformed or unknown codes must not resolve.
        assert!(locale_for_country("xx").is_none());
        assert!(locale_for_country("").is_none());
        assert!(locale_for_country("deu").is_none());
        assert!(locale_for_country("d1").is_none());
    }

    #[test]
    fn us_locale_matches_the_legacy_constant() {
        // The pre-existing hardcoded header value. `country=us` must therefore
        // change nothing but the clock.
        assert_eq!(
            locale_for_country("us").unwrap().accept_language,
            "en-US,en;q=0.9"
        );
    }

    #[test]
    fn cdp_accept_language_is_a_plain_tag_list() {
        let de = locale_for_country("de").unwrap();
        assert_eq!(de.cdp_accept_language(), "de-DE,de,en");
        let us = locale_for_country("us").unwrap();
        assert_eq!(us.cdp_accept_language(), "en-US,en");
    }

    #[test]
    fn js_languages_drops_q_values() {
        let de = locale_for_country("de").unwrap();
        assert_eq!(de.js_languages(), "['de-DE', 'de', 'en']");
        assert_eq!(de.primary_tag(), "de-DE");

        let us = locale_for_country("us").unwrap();
        assert_eq!(us.js_languages(), "['en-US', 'en']");
    }

    #[tokio::test]
    async fn request_locale_reads_the_task_local() {
        // Outside any scope the task-local is unset.
        assert!(request_locale().is_none());

        crate::REQUEST_COUNTRY
            .scope(Some("fr".to_string()), async {
                assert_eq!(request_locale().unwrap().timezone, "Europe/Paris");
            })
            .await;

        // A scoped-but-empty country stays on the default path.
        crate::REQUEST_COUNTRY
            .scope(None, async {
                assert!(request_locale().is_none());
            })
            .await;

        // An unknown country stays on the default path too.
        crate::REQUEST_COUNTRY
            .scope(Some("zz".to_string()), async {
                assert!(request_locale().is_none());
            })
            .await;
    }
}
