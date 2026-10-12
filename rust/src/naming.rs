//! MicroVM id generation: `kata-<pane>-<rand6>`, safe for containerd.

/// Builds a container id from a Herdr pane id.
pub fn microvm_id(pane_id: &str) -> String {
    let clean: String = pane_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = clean.trim_matches('-');
    let base: String = trimmed.chars().take(32).collect();
    let base = if base.is_empty() {
        "pane".to_string()
    } else {
        base
    };
    format!("kata-{base}-{}", rand_suffix())
}

fn rand_suffix() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut bytes = [0u8; 6];
    let random_ok = std::fs::File::open("/dev/urandom")
        .and_then(|mut file| {
            use std::io::Read as _;
            file.read_exact(&mut bytes).map(|_| ())
        })
        .is_ok();
    if !random_ok {
        // Fallback: time + pid. Not cryptographic, but unique enough for ids.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let pid = std::process::id();
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = ((nanos >> (i * 5)) ^ (pid << i)) as u8;
        }
    }
    bytes
        .iter()
        .map(|b| ALPHABET[(b % 36) as usize] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_safe_and_unique() {
        let first = microvm_id("a:1/b c");
        let second = microvm_id("a:1/b c");
        assert!(first.starts_with("kata-a-1-b-c-"));
        assert_ne!(first, second);
        assert!(first.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    }
}
