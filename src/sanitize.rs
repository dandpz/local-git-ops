//! Sanitization of repository-derived strings.
//!
//! Commit messages, author names and file paths are attacker-controlled when
//! scanning an untrusted repository. Control characters are stripped at
//! ingestion (`history`, `loc`) so neither the terminal renderer nor any
//! aggregation key can carry ANSI/OSC escape sequences; the Markdown exporter
//! additionally escapes Markdown metacharacters at the presentation layer.

/// Strip control characters (ANSI/OSC escape introducers included). Every
/// rendered value is a single-line cell, so newlines and tabs go too.
pub fn strip_controls(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

/// Neutralize Markdown table, code-span and inline-HTML metacharacters for
/// values interpolated into the exported report.
pub fn markdown(s: &str) -> String {
    strip_controls(s)
        .chars()
        .flat_map(|c| match c {
            '|' => vec!['\\', '|'],
            '<' => vec!['\\', '<'],
            // A backtick would terminate the surrounding code span; there is
            // no in-span escape, so substitute.
            '`' => vec!['\''],
            c => vec![c],
        })
        .collect()
}

/// Escape for interpolation into HTML text and attribute contexts.
pub fn html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in strip_controls(s).chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_ansi_and_osc_sequences() {
        assert_eq!(strip_controls("a\u{1b}[31mred\u{1b}[0mb"), "a[31mred[0mb");
        assert_eq!(strip_controls("t\u{1b}]0;pwned\u{7}"), "t]0;pwned");
        assert_eq!(strip_controls("multi\nline\r\ttext"), "multilinetext");
        assert_eq!(strip_controls("plain text"), "plain text");
    }

    #[test]
    fn escapes_html_metacharacters() {
        assert_eq!(html("<script>"), "&lt;script&gt;");
        assert_eq!(html(r#"a&b"c'd"#), "a&amp;b&quot;c&#39;d");
        assert_eq!(html("x\u{1b}[31m"), "x[31m");
        assert_eq!(html("src/main.rs"), "src/main.rs");
    }

    #[test]
    fn escapes_markdown_metacharacters() {
        assert_eq!(markdown("a|b"), "a\\|b");
        assert_eq!(markdown("<img src=x>"), "\\<img src=x>");
        assert_eq!(markdown("evil`code"), "evil'code");
        assert_eq!(markdown("src/main.rs"), "src/main.rs");
    }
}
