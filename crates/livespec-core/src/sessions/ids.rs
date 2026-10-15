//! Tab and window ids. A tab id becomes a file name (`tabs/<id>.json`), so
//! every id that arrives over IPC passes `valid_id` before it is used: that
//! check is what stops path traversal.

/// `^[tw]-[0-9a-f-]{1,40}$`.
pub fn valid_id(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("t-").or_else(|| id.strip_prefix("w-")) else { return false };
    (1..=40).contains(&rest.len()) && rest.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b'-')
}

/// `t-<unix millis, hex>-<per-process counter, hex>`.
pub fn tab_id(millis: u64, counter: u64) -> String {
    format!("t-{millis:x}-{counter:x}")
}

/// `w-<n>`, `n` in decimal (the workspace's `nextWindow`).
pub fn window_id(n: u64) -> String {
    format!("w-{n}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_id_accepts_only_our_ids() {
        let longest = format!("t-{}", "a".repeat(40));
        for ok in ["t-18f2a3b4c5d-1", "w-1", "w-12", "t-0", longest.as_str()] {
            assert!(valid_id(ok), "{ok}");
        }
        let too_long = format!("t-{}", "a".repeat(41));
        let bad = [
            "", "t-", "w-", "x-1", "T-1", "t-A1", "t-1/2", "t-../x", "t-1.json", "t- 1", "tt-1", "t_1", "t-1\n", "w-1;rm",
            too_long.as_str(),
        ];
        for id in bad {
            assert!(!valid_id(id), "{id:?}");
        }
    }

    #[test]
    fn generated_ids_pass_validation() {
        assert_eq!(tab_id(0x18f2a3b4c5d, 1), "t-18f2a3b4c5d-1");
        assert!(valid_id(&tab_id(u64::MAX, u64::MAX)), "even the longest");
        assert_eq!(window_id(3), "w-3");
        assert!(valid_id(&window_id(123)));
    }
}
