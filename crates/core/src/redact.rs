use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

const MAX_TEXT_BYTES: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedactionReport {
    pub text: String,
    pub redaction_count: usize,
    pub truncated: bool,
}

pub fn redact_text(input: &str) -> RedactionReport {
    static PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        vec![
            (
                Regex::new(r"(?i)\b(bearer)\s+[a-z0-9._~+/=-]{8,}").unwrap(),
                "$1 [REDACTED]",
            ),
            (
                Regex::new(r"(?i)\b(api[_-]?key|token|secret|password)\s*[:=]\s*[^\s,;]+").unwrap(),
                "$1=[REDACTED]",
            ),
            (
                Regex::new(r"\b(sk|xox[baprs]|gh[pousr])[-_][A-Za-z0-9_-]{8,}\b").unwrap(),
                "[REDACTED_TOKEN]",
            ),
            (
                Regex::new(r"(?i)(/Users/|[A-Z]:\\Users\\)[^/\\\s]+").unwrap(),
                "$1[USER]",
            ),
        ]
    });

    let mut output = input.to_owned();
    let mut count = 0;
    for (pattern, replacement) in patterns {
        count += pattern.find_iter(&output).count();
        output = pattern.replace_all(&output, *replacement).into_owned();
    }

    let mut truncated = false;
    if output.len() > MAX_TEXT_BYTES {
        let mut boundary = MAX_TEXT_BYTES;
        while !output.is_char_boundary(boundary) {
            boundary -= 1;
        }
        output.truncate(boundary);
        output.push_str("…[TRUNCATED]");
        truncated = true;
    }

    RedactionReport {
        text: output,
        redaction_count: count,
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_common_secrets_and_home_paths() {
        let report = redact_text(
            "Bearer abcdefghijkl token=hello-secret sk-test_123456789 /Users/alice/code",
        );
        assert!(!report.text.contains("abcdefghijkl"));
        assert!(!report.text.contains("hello-secret"));
        assert!(!report.text.contains("123456789"));
        assert!(report.text.contains("/Users/[USER]/code"));
        assert!(report.redaction_count >= 4);
    }

    #[test]
    fn truncates_large_text_on_utf8_boundary() {
        let report = redact_text(&"测".repeat(5000));
        assert!(report.truncated);
        assert!(report.text.ends_with("[TRUNCATED]"));
    }
}
