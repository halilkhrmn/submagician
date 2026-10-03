//! "Report a problem": a plain-text report the user reads first, then sends as a GitHub issue or
//! by e-mail. Links cannot carry much text (browsers and mail programs cut long URLs), so the
//! full report is saved to a file and the link carries the summary plus the newest log lines.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::{APP_REPO, SUPPORT_EMAIL, VERSION};

/// A GitHub "new issue" link longer than this is rejected, so the body is trimmed to fit.
const MAX_ISSUE_URL: usize = 7_500;
/// Some mail programs (and Windows' URL handling) cut `mailto:` links around 2 000 characters.
const MAX_MAILTO_URL: usize = 1_800;

pub struct Report {
    /// Version, system and the app's state.
    pub summary: String,
    /// Log lines, oldest first.
    pub log: Vec<String>,
}

impl Report {
    /// A report with the version, the system, `details` (one "Name: value" per line) and `log`.
    pub fn new(details: &[(&str, String)], log: Vec<String>) -> Report {
        let mut summary = format!("SubMagician {VERSION}\nSystem: {} ({})\n", os_description(), std::env::consts::ARCH);
        for (name, value) in details {
            let _ = writeln!(summary, "{name}: {value}");
        }
        Report { summary, log }
    }

    pub fn text(&self, note: &str) -> String {
        let mut out = String::new();
        if !note.trim().is_empty() {
            let _ = writeln!(out, "What happened: {}\n", note.trim());
        }
        out.push_str(&self.summary);
        let _ = writeln!(out, "\nRecent log ({} lines):", self.log.len());
        for line in &self.log {
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    /// Saves the full report into `dir` and returns its path.
    pub fn save(&self, dir: &Path, note: &str) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!("submagician-report-{}.txt", chrono::Local::now().format("%Y%m%d-%H%M%S")));
        std::fs::write(&path, self.text(note))?;
        Ok(path)
    }

    pub fn github_url(&self, note: &str, saved: Option<&Path>) -> String {
        let title = match note.trim() {
            "" => "Problem report".to_owned(),
            n => n.chars().take(80).collect(),
        };
        let base = format!("https://github.com/{APP_REPO}/issues/new?title={}&body=", encode(&title));
        let attach = saved
            .map(|p| format!("\n_Please drag the full report into this issue: `{}`_\n", p.display()))
            .unwrap_or_default();
        let fit = |lines: usize| {
            let start = self.log.len().saturating_sub(lines);
            format!(
                "{}\n```\n{}```\n{attach}\n**Newest log lines**\n```\n{}\n```\n",
                if note.trim().is_empty() {
                    "_Describe what happened here._\n".to_owned()
                } else {
                    format!("{}\n", note.trim())
                },
                self.summary,
                self.log[start..].join("\n")
            )
        };
        base.clone() + &encode(&longest_fitting(&base, MAX_ISSUE_URL, self.log.len(), fit))
    }

    pub fn mailto_url(&self, note: &str, saved: Option<&Path>) -> String {
        let base = format!("mailto:{SUPPORT_EMAIL}?subject={}&body=", encode("SubMagician problem report"));
        let attach = saved.map(|p| format!("Please attach the full report: {}\n\n", p.display())).unwrap_or_default();
        let fit = |lines: usize| {
            let start = self.log.len().saturating_sub(lines);
            format!(
                "{}\n\n{attach}{}\nNewest log lines:\n{}\n",
                note.trim(),
                self.summary,
                self.log[start..].join("\n")
            )
        };
        base.clone() + &encode(&longest_fitting(&base, MAX_MAILTO_URL, self.log.len(), fit))
    }
}

/// The body with as many of the newest log lines as fit into `max` URL characters.
fn longest_fitting(base: &str, max: usize, lines: usize, body: impl Fn(usize) -> String) -> String {
    (0..=lines)
        .rev()
        .map(&body)
        .find(|b| base.len() + encode(b).len() <= max)
        .unwrap_or_else(|| body(0).chars().take(max / 4).collect())
}

/// Percent-encoding for URL query values (RFC 3986 unreserved characters stay as they are).
pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

/// "Windows 11 24H2 (build 26100)", "Ubuntu 24.04 LTS", …
pub fn os_description() -> String {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let value = |name: &str| -> String {
            std::process::Command::new("reg")
                .args(["query", r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion", "/v", name])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
                .ok()
                .and_then(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .lines()
                        .find(|l| l.trim_start().starts_with(name))
                        .and_then(|l| l.split_whitespace().nth(2).map(str::to_owned))
                })
                .unwrap_or_default()
        };
        let build = value("CurrentBuildNumber");
        let display = value("DisplayVersion");
        // ProductName still says "Windows 10" on Windows 11; the build number tells.
        let name = if build.parse::<u32>().is_ok_and(|b| b >= 22000) { "Windows 11" } else { "Windows 10" };
        if build.is_empty() { "Windows".to_owned() } else { format!("{name} {display} (build {build})") }
    }
    #[cfg(target_os = "macos")]
    {
        let version = std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_default();
        format!("macOS {version}")
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let name = std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|t| {
                t.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=")).map(|v| v.trim_matches('"').to_owned())
            })
            .unwrap_or_else(|| std::env::consts::OS.to_owned());
        let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
        if session.is_empty() { name } else { format!("{name}, {session}") }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(lines: usize) -> Report {
        Report::new(
            &[("Folder", "3 videos".into())],
            (0..lines)
                .map(|i| format!("2026-01-01 12:00:{:02} INFO  [engine] line {i} with ünïcode & ?=#", i % 60))
                .collect(),
        )
    }

    #[test]
    fn links_fit_and_keep_the_newest_lines() {
        let r = report(500);
        assert!(r.summary.starts_with(&format!("SubMagician {VERSION}\nSystem: ")));
        assert!(r.summary.contains("Folder: 3 videos"));
        let issue = r.github_url("Sync froze", None);
        assert!(issue.len() <= MAX_ISSUE_URL, "{}", issue.len());
        assert!(issue.starts_with(&format!("https://github.com/{APP_REPO}/issues/new?title=Sync%20froze&body=")));
        assert!(issue.contains(&encode("line 499 ")), "newest line kept");
        assert!(!issue.contains(&encode("line 0 ")), "oldest lines dropped");

        let mail = r.mailto_url("", Some(Path::new("/tmp/r.txt")));
        assert!(mail.len() <= MAX_MAILTO_URL, "{}", mail.len());
        assert!(mail.starts_with("mailto:halilkahraman@yandex.com?subject=SubMagician%20problem%20report&body="));
        assert!(mail.contains(&encode("/tmp/r.txt")));
    }

    #[test]
    fn saves_the_full_text() {
        let dir = std::env::temp_dir().join(format!("submagician-report-{}", std::process::id()));
        let path = report(3).save(&dir, "  it froze ").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("What happened: it froze\n\nSubMagician "));
        assert!(text.contains("Recent log (3 lines):\n") && text.trim_end().ends_with("line 2 with ünïcode & ?=#"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn encodes_reserved_characters() {
        assert_eq!(encode("a b&c=d?é"), "a%20b%26c%3Dd%3F%C3%A9");
    }
}
