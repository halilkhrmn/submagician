//! Turning downloaded subtitle bytes into clean UTF-8 text.
//!
//! Turkish subtitles are often Windows-1254 (or ISO-8859-9); read as anything else, ı ş ğ İ
//! turn into garbage. The language we asked for is the best hint for the detector.

use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};
use encoding_rs::{Encoding, UTF_8};

use crate::lang;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub text: String,
    /// Name of the encoding the bytes were in ("UTF-8", "windows-1254", …).
    pub encoding: &'static str,
}

/// Decodes subtitle bytes. `language` is the code of the language the subtitle is in.
pub fn decode(bytes: &[u8], language: Option<&str>) -> Decoded {
    let (encoding, body) = match Encoding::for_bom(bytes) {
        Some((enc, bom_len)) => (enc, &bytes[bom_len..]),
        None if std::str::from_utf8(bytes).is_ok() => (UTF_8, bytes),
        None => (detect(bytes, language), bytes),
    };
    let (text, _) = encoding.decode_without_bom_handling(body);
    Decoded { text: normalize_newlines(&text), encoding: encoding.name() }
}

fn detect(bytes: &[u8], language: Option<&str>) -> &'static Encoding {
    let tld = language.and_then(lang::find).and_then(|l| l.tld);
    let mut detector = EncodingDetector::new(Iso2022JpDetection::Deny);
    detector.feed(bytes, true);
    let guess = detector.guess(tld.map(str::as_bytes), Utf8Detection::Allow);
    // For Turkish, Latin-1 style guesses are almost always cp1254 misread: same bytes for
    // most letters, but 0xFD/0xFE/0xF0/0xDD are ı/ş/ğ/İ there.
    if language == Some("tr") && (guess == encoding_rs::WINDOWS_1252 || guess == encoding_rs::ISO_8859_2) {
        return encoding_rs::WINDOWS_1254;
    }
    guess
}

fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Subtitle text format, judged from the content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Srt,
    Ass,
    Ssa,
    Vtt,
    MicroDvd,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Srt => "srt",
            Format::Ass => "ass",
            Format::Ssa => "ssa",
            Format::Vtt => "vtt",
            Format::MicroDvd => "sub",
        }
    }
}

pub fn detect_format(text: &str) -> Format {
    let head: String = text.trim_start().chars().take(2000).collect();
    if head.starts_with("WEBVTT") {
        Format::Vtt
    } else if head.starts_with("[Script Info]") {
        if head.contains("ScriptType: v4.00+") || head.contains("[V4+ Styles]") { Format::Ass } else { Format::Ssa }
    } else if head.starts_with('{') && head.lines().next().is_some_and(is_microdvd_line) {
        Format::MicroDvd
    } else {
        Format::Srt
    }
}

fn is_microdvd_line(line: &str) -> bool {
    let mut parts = line.splitn(3, '}');
    let a = parts.next().and_then(|p| p.strip_prefix('{'));
    let b = parts.next().and_then(|p| p.strip_prefix('{'));
    matches!((a, b), (Some(a), Some(b)) if a.chars().all(|c| c.is_ascii_digit()) && b.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TR_SAMPLE: &str = "1\r\n00:00:01,000 --> 00:00:03,000\r\nIşık ağır geliyor, Şule.\r\n\r\n2\r\n00:00:04,000 --> 00:00:06,000\r\nİstanbul'a gidiyoruz, çocuğu da götür.\r\n";

    #[test]
    fn keeps_utf8_and_strips_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(TR_SAMPLE.as_bytes());
        let d = decode(&bytes, Some("tr"));
        assert_eq!(d.encoding, "UTF-8");
        assert!(d.text.starts_with("1\n00:00:01,000"));
        assert!(!d.text.contains('\r'));
    }

    #[test]
    fn fixes_windows_1254_turkish() {
        let (bytes, _, _) = encoding_rs::WINDOWS_1254.encode(TR_SAMPLE);
        let d = decode(&bytes, Some("tr"));
        assert_eq!(d.encoding, "windows-1254");
        assert!(d.text.contains("Işık ağır geliyor, Şule."), "{}", d.text);
        assert!(d.text.contains("İstanbul'a"));
    }

    #[test]
    fn decodes_utf16() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in TR_SAMPLE.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        assert!(decode(&bytes, None).text.contains("götür"));
    }

    #[test]
    fn detects_formats() {
        assert_eq!(detect_format(TR_SAMPLE), Format::Srt);
        assert_eq!(detect_format("WEBVTT\n\n00:01.000 --> 00:02.000\nhi"), Format::Vtt);
        assert_eq!(detect_format("[Script Info]\nScriptType: v4.00+\n"), Format::Ass);
        assert_eq!(detect_format("{100}{200}Hello|world"), Format::MicroDvd);
    }
}
