//! Subtitle timings: read the cue times of SRT, WebVTT and ASS/SSA text, change them, and write
//! the text back with everything else (styles, tags, positions, comments) untouched.

use crate::text::Format;
use crate::{Error, Result};

/// One cue: where its time line is and the parts around the two times.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cue {
    pub start: i64,
    pub end: i64,
    line: usize,
    head: String,
    sep: String,
    tail: String,
}

/// A subtitle file split into lines, with its cues' times parsed (milliseconds).
#[derive(Debug, Clone)]
pub struct Document {
    pub format: Format,
    lines: Vec<String>,
    pub cues: Vec<Cue>,
}

impl Document {
    pub fn parse(text: &str) -> Result<Document> {
        let format = crate::text::detect_format(text);
        let lines: Vec<String> = text.lines().map(str::to_owned).collect();
        let cues: Vec<Cue> = match format {
            Format::Srt | Format::Vtt => lines.iter().enumerate().filter_map(|(i, l)| arrow_cue(i, l)).collect(),
            Format::Ass | Format::Ssa => lines.iter().enumerate().filter_map(|(i, l)| dialogue_cue(i, l)).collect(),
            Format::MicroDvd => {
                return Err(Error::Parse("MicroDVD (.sub) subtitles count frames and cannot be synced".into()));
            }
        };
        if cues.is_empty() {
            return Err(Error::Parse("no subtitle lines with times found".into()));
        }
        Ok(Document { format, lines, cues })
    }

    /// Changes every cue time with `f` (negative results become 0).
    pub fn map_times(&mut self, mut f: impl FnMut(usize, i64) -> i64) {
        for (i, cue) in self.cues.iter_mut().enumerate() {
            cue.start = f(i, cue.start).max(0);
            cue.end = f(i, cue.end).max(cue.start);
        }
    }

    pub fn shift(&mut self, ms: i64) {
        self.map_times(|_, t| t + ms);
    }

    pub fn render(&self) -> String {
        let mut lines = self.lines.clone();
        for cue in &self.cues {
            let (start, end) = match self.format {
                Format::Ass | Format::Ssa => (fmt_ass(cue.start), fmt_ass(cue.end)),
                Format::Vtt => (fmt_clock(cue.start, '.'), fmt_clock(cue.end, '.')),
                _ => (fmt_clock(cue.start, ','), fmt_clock(cue.end, ',')),
            };
            lines[cue.line] = format!("{}{start}{}{end}{}", cue.head, cue.sep, cue.tail);
        }
        let mut out = lines.join("\n");
        out.push('\n');
        out
    }
}

/// `00:01:02,345 --> 00:01:04,000 [settings]` (SRT and WebVTT).
fn arrow_cue(line: usize, text: &str) -> Option<Cue> {
    let (left, right) = text.split_once("-->")?;
    let start = parse_clock(left.trim())?;
    let right_trimmed = right.trim_start();
    let end_len = right_trimmed.find(char::is_whitespace).unwrap_or(right_trimmed.len());
    let end = parse_clock(&right_trimmed[..end_len])?;
    let lead = text.len() - text.trim_start().len();
    Some(Cue {
        start,
        end,
        line,
        head: text[..lead].to_owned(),
        sep: " --> ".into(),
        tail: right_trimmed[end_len..].to_owned(),
    })
}

/// `HH:MM:SS,mmm`, `H:MM:SS.mmm`, `MM:SS.mmm`; fractions of any length.
fn parse_clock(s: &str) -> Option<i64> {
    let (clock, frac) = match s.rsplit_once([',', '.']) {
        Some((c, f)) if !f.is_empty() && f.chars().all(|c| c.is_ascii_digit()) => (c, f),
        _ => (s, ""),
    };
    let parts: Vec<i64> = clock.split(':').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
    let seconds = match parts.as_slice() {
        [h, m, s] => h * 3600 + m * 60 + s,
        [m, s] => m * 60 + s,
        _ => return None,
    };
    let ms = match frac.len() {
        0 => 0,
        n => {
            let digits: i64 = frac[..n.min(3)].parse().ok()?;
            digits * 10i64.pow(3 - n.min(3) as u32)
        }
    };
    Some(seconds * 1000 + ms)
}

pub(crate) fn fmt_clock(ms: i64, sep: char) -> String {
    let (h, rest) = (ms / 3_600_000, ms % 3_600_000);
    format!("{h:02}:{:02}:{:02}{sep}{:03}", rest / 60_000, rest / 1000 % 60, rest % 1000)
}

/// `Dialogue: 0,0:01:02.34,0:01:04.00,Default,...` — Start and End are the 2nd and 3rd fields.
fn dialogue_cue(line: usize, text: &str) -> Option<Cue> {
    let colon = text.find(':')?;
    if !text[..colon].trim().eq_ignore_ascii_case("dialogue") {
        return None;
    }
    let mut commas = text.match_indices(',').map(|(i, _)| i);
    let (c1, c2, c3) = (commas.next()?, commas.next()?, commas.next()?);
    let start = parse_clock(text[c1 + 1..c2].trim())?;
    let end = parse_clock(text[c2 + 1..c3].trim())?;
    Some(Cue { start, end, line, head: text[..=c1].to_owned(), sep: ",".into(), tail: text[c3..].to_owned() })
}

/// `H:MM:SS.cc` (centiseconds).
fn fmt_ass(ms: i64) -> String {
    let cs = (ms + 5) / 10;
    format!("{}:{:02}:{:02}.{:02}", cs / 360_000, cs / 6000 % 60, cs / 100 % 60, cs % 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRT: &str =
        "1\n00:00:01,000 --> 00:00:02,500\n<i>Merhaba</i>\n\n2\n00:01:00,05 --> 00:01:02,000 X1:10\nİkinci satır\n";

    #[test]
    fn parses_and_renders_srt_unchanged() {
        let doc = Document::parse(SRT).unwrap();
        assert_eq!(doc.cues.len(), 2);
        assert_eq!((doc.cues[0].start, doc.cues[0].end), (1000, 2500));
        assert_eq!(doc.cues[1].start, 60_050, "two-digit fraction is hundredths");
        let out = doc.render();
        assert!(out.contains("00:01:00,050 --> 00:01:02,000 X1:10\nİkinci satır"));
        assert!(out.contains("<i>Merhaba</i>"));
    }

    #[test]
    fn shifts_and_clamps() {
        let mut doc = Document::parse(SRT).unwrap();
        doc.shift(-1500);
        assert_eq!((doc.cues[0].start, doc.cues[0].end), (0, 1000));
        assert!(doc.render().starts_with("1\n00:00:00,000 --> 00:00:01,000\n"));
    }

    #[test]
    fn handles_vtt() {
        let vtt = "WEBVTT\n\nNOTE x\n\n00:05.000 --> 00:06.250 align:start\nHi\n\n01:00:00.000 --> 01:00:01.000\nBye\n";
        let mut doc = Document::parse(vtt).unwrap();
        assert_eq!((doc.cues[0].start, doc.cues[0].end), (5000, 6250));
        doc.shift(1000);
        let out = doc.render();
        assert!(out.contains("00:00:06.000 --> 00:00:07.250 align:start\nHi"), "{out}");
        assert!(out.contains("01:00:01.000 --> 01:00:02.000\nBye"));
    }

    #[test]
    fn handles_ass() {
        let ass = "[Script Info]\nScriptType: v4.00+\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.50,0:00:03.00,Default,,0,0,0,,{\\i1}Selam, dünya{\\i0}\nComment: 0,0:00:09.00,0:00:10.00,Default,,0,0,0,,note\n";
        let mut doc = Document::parse(ass).unwrap();
        assert_eq!(doc.cues.len(), 1, "comments are not cues");
        assert_eq!((doc.cues[0].start, doc.cues[0].end), (1500, 3000));
        doc.shift(61_000);
        let out = doc.render();
        assert!(out.contains("Dialogue: 0,0:01:02.50,0:01:04.00,Default,,0,0,0,,{\\i1}Selam, dünya{\\i0}"), "{out}");
        assert!(out.contains("Comment: 0,0:00:09.00"));
    }

    #[test]
    fn rejects_microdvd_and_empty() {
        assert!(Document::parse("{10}{20}Hi").is_err());
        assert!(Document::parse("just text").is_err());
    }
}
