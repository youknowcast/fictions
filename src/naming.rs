pub fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

pub fn output_filename(base: &str, hash8: &str, ext: &str) -> String {
    format!("{}-{}.{}", truncate_chars(base, 50), hash8, ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_by_chars_not_bytes() {
        let s = "あ".repeat(60);
        assert_eq!(truncate_chars(&s, 50).chars().count(), 50);
    }

    #[test]
    fn keeps_short_names() {
        assert_eq!(truncate_chars("hello", 50), "hello");
    }

    #[test]
    fn truncates_emoji_without_panic() {
        let s = "🎬".repeat(60);
        assert_eq!(truncate_chars(&s, 50).chars().count(), 50);
    }

    #[test]
    fn builds_name_with_hash() {
        assert_eq!(
            output_filename("hello", "deadbeef", "mp4"),
            "hello-deadbeef.mp4"
        );
    }
}
