//! "What's new": after an update, the first start shows the release notes of every version since
//! the one that ran before. The notes are built in (`changelog/en.md`), so they work offline.

const NOTES: &str = include_str!("../../../changelog/en.md");

/// Versions shown at most, newest first: someone who skipped many releases gets the latest ones.
const MAX_VERSIONS: usize = 3;

type Version = (u32, u32, u32);

fn parse_version(v: &str) -> Option<Version> {
    let mut parts = v.trim().trim_start_matches(['v', 'V']).split('.');
    let mut next = || parts.next()?.parse().ok();
    Some((next()?, next()?, next()?))
}

/// `## <version>` sections of a changelog, in file order (newest first).
fn sections(text: &str) -> Vec<(Version, String)> {
    let mut out = Vec::new();
    let mut current: Option<(Version, String)> = None;
    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            out.extend(current.take());
            current = parse_version(heading.split_whitespace().next().unwrap_or("")).map(|v| (v, String::new()));
        } else if let Some((_, body)) = current.as_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    out.extend(current);
    out
}

/// Markdown bullets as plain text: "- " becomes "•  " and wrapped lines join their bullet.
fn plain(body: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in body.trim().lines() {
        let trimmed = line.trim_start();
        match trimmed.strip_prefix("- ") {
            Some(rest) => lines.push(format!("•  {rest}")),
            None if line.starts_with("  ") && !lines.is_empty() => {
                let last = lines.last_mut().unwrap();
                last.push(' ');
                last.push_str(trimmed);
            }
            None => lines.push(line.to_owned()),
        }
    }
    lines.join("\n")
}

/// The notes of the versions after `last` up to `current`, as plain text, or `None` when there is
/// nothing to show. An empty `last` (first start) shows only the current version.
pub fn notes_since(text: &str, last: &str, current: &str) -> Option<String> {
    let current = parse_version(current)?;
    let last = parse_version(last);
    if last.is_some_and(|l| l >= current) {
        return None;
    }
    let shown: Vec<_> = sections(text)
        .into_iter()
        .filter(|(v, _)| *v <= current && last.map_or(*v == current, |l| *v > l))
        .take(MAX_VERSIONS)
        .collect();
    // The dialog's title names the current version; more than one gets a heading each.
    let headings = shown.len() > 1;
    let shown: Vec<String> = shown
        .into_iter()
        .map(|((a, b, c), body)| if headings { format!("{a}.{b}.{c}\n{}", plain(&body)) } else { plain(&body) })
        .collect();
    (!shown.is_empty()).then(|| shown.join("\n\n"))
}

/// What to show at this start, given the version that ran before.
pub fn pending(last: &str) -> Option<String> {
    notes_since(NOTES, last, crate::VERSION)
}

/// The notes of this version, for "What's new" in Settings.
pub fn current() -> String {
    notes_since(NOTES, "", crate::VERSION).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str =
        "# Changelog\n\n## 0.3.0\n- New A\n- New B that is\n  wrapped\n\n## 0.2.9\nFix C\n\n## 0.2.8\n- D\n";

    #[test]
    fn notes_since_last_start() {
        assert_eq!(
            notes_since(LOG, "0.2.8", "0.3.0").unwrap(),
            "0.3.0\n•  New A\n•  New B that is wrapped\n\n0.2.9\nFix C"
        );
        assert_eq!(notes_since(LOG, "", "0.3.0").unwrap(), "•  New A\n•  New B that is wrapped");
        assert_eq!(notes_since(LOG, "0.3.0", "0.3.0"), None);
        assert_eq!(notes_since(LOG, "0.3.1", "0.3.0"), None, "went back a version");
        assert_eq!(notes_since(LOG, "0.2.9", "0.3.1"), Some("•  New A\n•  New B that is wrapped".into()));
        assert_eq!(notes_since(LOG, "", "0.4.0"), None, "no entry for this version");
    }

    #[test]
    fn changelog_has_the_current_version() {
        assert!(!current().is_empty(), "changelog/en.md has no section for {}", crate::VERSION);
    }
}
