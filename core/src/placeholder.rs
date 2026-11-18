//! Snippet placeholders (design section 8.3), resolved at paste time.
//!
//! | Token | Resolves to |
//! |---|---|
//! | `{date}` / `{date:%Y-%m-%d}` | today's date |
//! | `{time}` / `{time:%H:%M:%S}` | current time |
//! | `{clipboard}` / `{clipboard:1}` | current clipboard text / nth previous clip |
//! | `{cursor}` | where the caret ends up after paste |
//!
//! Unknown tokens are left as literal text.

use chrono::format::{Item, StrftimeItems};
use chrono::NaiveDateTime;

pub const DEFAULT_DATE_FORMAT: &str = "%Y-%m-%d";
pub const DEFAULT_TIME_FORMAT: &str = "%H:%M";

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Expanded {
    pub text: String,
    /// Number of ← presses to send after pasting so the caret lands where
    /// `{cursor}` was. `None` if the snippet had no `{cursor}`.
    pub cursor_back: Option<usize>,
}

/// Expand placeholders. `clipboard(n)` returns the nth clip text, where 0 is
/// the current clipboard and 1 is the previous clip.
pub fn expand(body: &str, now: NaiveDateTime, clipboard: &dyn Fn(usize) -> Option<String>) -> Expanded {
    let mut out = String::with_capacity(body.len());
    // Byte offset in `out` of the first {cursor}.
    let mut cursor_at: Option<usize> = None;
    let mut rest = body;

    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let token_end = after.find(['{', '}']).filter(|i| after.as_bytes()[*i] == b'}');
        let Some(close) = token_end else {
            out.push('{');
            rest = after;
            continue;
        };
        let token = &after[..close];
        let (name, arg) = match token.split_once(':') {
            Some((n, a)) => (n, Some(a)),
            None => (token, None),
        };
        let replacement = match (name, arg) {
            ("date", fmt) => format_time(now, fmt.unwrap_or(DEFAULT_DATE_FORMAT)),
            ("time", fmt) => format_time(now, fmt.unwrap_or(DEFAULT_TIME_FORMAT)),
            ("clipboard", None) => Some(clipboard(0).unwrap_or_default()),
            ("clipboard", Some(n)) => n.parse::<usize>().ok().map(|n| clipboard(n).unwrap_or_default()),
            ("cursor", None) => {
                cursor_at.get_or_insert(out.len());
                Some(String::new())
            }
            _ => None,
        };
        match replacement {
            Some(text) => out.push_str(&text),
            None => {
                out.push('{');
                out.push_str(token);
                out.push('}');
            }
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);

    let cursor_back = cursor_at.map(|at| out[at..].chars().count());
    Expanded { text: out, cursor_back }
}

/// `None` if the format string is invalid, so the token stays literal.
fn format_time(now: NaiveDateTime, fmt: &str) -> Option<String> {
    let items: Vec<Item<'_>> = StrftimeItems::new(fmt).collect();
    if items.iter().any(|i| matches!(i, Item::Error)) {
        return None;
    }
    use std::fmt::Write;
    let mut s = String::new();
    write!(s, "{}", now.format_with_items(items.iter())).ok()?;
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn now() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 17).unwrap().and_hms_opt(9, 5, 30).unwrap()
    }

    fn clip(n: usize) -> Option<String> {
        match n {
            0 => Some("current".into()),
            1 => Some("previous".into()),
            _ => None,
        }
    }

    fn run(body: &str) -> Expanded {
        expand(body, now(), &clip)
    }

    #[test]
    fn plain_text_is_untouched() {
        assert_eq!(run("hello world"), Expanded { text: "hello world".into(), cursor_back: None });
    }

    #[test]
    fn date_and_time_defaults() {
        assert_eq!(run("{date} {time}").text, "2026-11-17 09:05");
    }

    #[test]
    fn date_and_time_formats() {
        assert_eq!(run("{date:%d/%m/%Y}").text, "17/11/2026");
        assert_eq!(run("{time:%H:%M:%S}").text, "09:05:30");
        assert_eq!(run("{date:%A}").text, "Tuesday");
    }

    #[test]
    fn invalid_format_stays_literal() {
        assert_eq!(run("{date:%Q}").text, "{date:%Q}");
    }

    #[test]
    fn clipboard_tokens() {
        assert_eq!(run("[{clipboard}] [{clipboard:1}] [{clipboard:7}]").text, "[current] [previous] []");
        assert_eq!(run("{clipboard:x}").text, "{clipboard:x}");
    }

    #[test]
    fn clipboard_content_is_not_re_expanded() {
        let c = |_: usize| Some("{date}".to_string());
        assert_eq!(expand("{clipboard}", now(), &c).text, "{date}");
    }

    #[test]
    fn cursor_counts_characters_after_it() {
        let e = run("Thanks,\n{cursor}\n— A");
        assert_eq!(e.text, "Thanks,\n\n— A");
        assert_eq!(e.cursor_back, Some(4));
        assert_eq!(run("end{cursor}").cursor_back, Some(0));
    }

    #[test]
    fn cursor_accounts_for_later_expansions() {
        let e = run("{cursor}{date}");
        assert_eq!(e.cursor_back, Some(10));
    }

    #[test]
    fn only_first_cursor_counts() {
        let e = run("a{cursor}b{cursor}c");
        assert_eq!(e.text, "abc");
        assert_eq!(e.cursor_back, Some(2));
    }

    #[test]
    fn unknown_and_malformed_tokens_are_literal() {
        assert_eq!(run("{nope} {x:y} {} {cursor:1}").text, "{nope} {x:y} {} {cursor:1}");
        assert_eq!(run("fn main() { let a = {date}; }").text, "fn main() { let a = 2026-11-17; }");
        assert_eq!(run("{unclosed").text, "{unclosed");
        assert_eq!(run("{\"json\": {\"k\": 1}}").text, "{\"json\": {\"k\": 1}}");
    }
}
