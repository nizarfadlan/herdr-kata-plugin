//! Small helpers: shell quoting.

/// Quotes one argv word for POSIX `sh`.
pub fn shell_quote(word: &str) -> String {
    if word.is_empty() {
        return "''".to_string();
    }
    let safe = word
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_./:=,".contains(c));
    if safe {
        return word.to_string();
    }
    format!("'{}'", word.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_rules() {
        assert_eq!(shell_quote("simple"), "simple");
        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }
}
