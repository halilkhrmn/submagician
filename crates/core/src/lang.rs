//! Subtitle languages. Codes are the lowercase ones OpenSubtitles uses (ISO 639-1, plus
//! `pt-br`, `zh-cn`, `zh-tw`); `alpha3` (ISO 639-2/B) is what MKV tracks and many sidecar files use.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Language {
    pub code: &'static str,
    pub alpha3: &'static str,
    pub name: &'static str,
    /// Top-level domain used as a hint for legacy encoding detection.
    pub tld: Option<&'static str>,
}

const fn l(code: &'static str, alpha3: &'static str, name: &'static str, tld: Option<&'static str>) -> Language {
    Language { code, alpha3, name, tld }
}

pub const LANGUAGES: &[Language] = &[
    l("tr", "tur", "Turkish", Some("tr")),
    l("en", "eng", "English", None),
    l("de", "ger", "German", Some("de")),
    l("fr", "fre", "French", Some("fr")),
    l("es", "spa", "Spanish", Some("es")),
    l("it", "ita", "Italian", Some("it")),
    l("pt-pt", "por", "Portuguese", Some("pt")),
    l("pt-br", "pob", "Portuguese (Brazil)", Some("br")),
    l("nl", "dut", "Dutch", Some("nl")),
    l("pl", "pol", "Polish", Some("pl")),
    l("cs", "cze", "Czech", Some("cz")),
    l("hu", "hun", "Hungarian", Some("hu")),
    l("ro", "rum", "Romanian", Some("ro")),
    l("bg", "bul", "Bulgarian", Some("bg")),
    l("el", "gre", "Greek", Some("gr")),
    l("ru", "rus", "Russian", Some("ru")),
    l("uk", "ukr", "Ukrainian", Some("ua")),
    l("ar", "ara", "Arabic", Some("sa")),
    l("fa", "per", "Persian", Some("ir")),
    l("he", "heb", "Hebrew", Some("il")),
    l("az", "aze", "Azerbaijani", Some("az")),
    l("sv", "swe", "Swedish", Some("se")),
    l("da", "dan", "Danish", Some("dk")),
    l("no", "nor", "Norwegian", Some("no")),
    l("fi", "fin", "Finnish", Some("fi")),
    l("zh-cn", "chi", "Chinese (simplified)", Some("cn")),
    l("zh-tw", "zht", "Chinese (traditional)", Some("tw")),
    l("ja", "jpn", "Japanese", Some("jp")),
    l("ko", "kor", "Korean", Some("kr")),
];

/// Looks a language up by its code, its ISO 639-2 codes or its English name (case-insensitive).
pub fn find(tag: &str) -> Option<&'static Language> {
    let tag = tag.trim().to_ascii_lowercase();
    if tag.is_empty() {
        return None;
    }
    LANGUAGES.iter().find(|lang| {
        lang.code == tag
            || lang.alpha3 == tag
            || lang.name.eq_ignore_ascii_case(&tag)
            || alias(lang.code).contains(&tag.as_str())
    })
}

fn alias(code: &str) -> &'static [&'static str] {
    match code {
        "tr" => &["trk", "turkce", "türkçe"],
        "de" => &["deu"],
        "fr" => &["fra"],
        "nl" => &["nld"],
        "cs" => &["ces"],
        "ro" => &["ron"],
        "el" => &["ell"],
        "fa" => &["fas", "farsi"],
        "pt-pt" => &["pt"],
        "pt-br" => &["ptbr", "pt_br", "brazilian"],
        "zh-cn" => &["zh", "zho", "chs"],
        "zh-tw" => &["cht"],
        _ => &[],
    }
}

/// Parses a comma or space separated list ("tr, en") into known codes, keeping order, no repeats.
pub fn parse_list(list: &str) -> Vec<&'static str> {
    let mut out = Vec::new();
    for tag in list.split([',', ' ', ';']).filter(|t| !t.is_empty()) {
        if let Some(lang) = find(tag)
            && !out.contains(&lang.code)
        {
            out.push(lang.code);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_by_any_tag() {
        assert_eq!(find("TR").unwrap().code, "tr");
        assert_eq!(find("tur").unwrap().code, "tr");
        assert_eq!(find("turkish").unwrap().code, "tr");
        assert_eq!(find("Türkçe").unwrap().code, "tr");
        assert_eq!(find("pob").unwrap().code, "pt-br");
        assert!(find("xx").is_none());
        assert!(find("").is_none());
    }

    #[test]
    fn parses_lists() {
        assert_eq!(parse_list("tr, en,TR;eng xx"), vec!["tr", "en"]);
    }
}
