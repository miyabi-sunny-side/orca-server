// Adapted from scad-live/client/src/lib/fuzzy.js (MIT, see LICENSE).
// Keep ranking here; clients consume the API order rather than reimplement it.
pub fn score(query: &str, text: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    let query = query.to_lowercase();
    let text = text.to_lowercase();
    let chars: Vec<_> = text.chars().collect();
    let base = chars.iter().rposition(|c| *c == '/').map_or(0, |i| i + 1);
    let mut wanted = query.chars().peekable();
    let mut previous = None;
    let mut result = 0;
    for (index, character) in chars.iter().enumerate() {
        if wanted.peek() != Some(character) {
            continue;
        }
        wanted.next();
        result += 100;
        if previous == index.checked_sub(1) && previous.is_some() {
            result += 500;
        }
        if index == 0 || matches!(chars[index - 1], '/' | '-' | '_') {
            result += 300;
        }
        if index == base {
            result += 400;
        }
        previous = Some(index);
        if wanted.peek().is_none() {
            break;
        }
    }
    if wanted.peek().is_some() {
        return None;
    }
    if text.rsplit('/').next().unwrap_or_default().contains(&query) {
        result += 1000;
    }
    Some(result - i64::try_from(chars.len()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::score;

    #[test]
    fn subsequences_are_case_insensitive_and_misses_are_excluded() {
        assert!(score("bx", "Box.stl").is_some());
        assert!(score("zzz", "box.stl").is_none());
        assert!(score("机", "家具/机.stl").is_some());
        assert_eq!(score("", "anything"), Some(0));
    }

    #[test]
    fn basename_and_short_contiguous_matches_rank_first() {
        let mut paths = ["deep/nested/part-holder.stl", "deep/part.stl", "part.stl"];
        paths.sort_by_key(|path| std::cmp::Reverse(score("part", path)));
        assert_eq!(
            paths,
            ["part.stl", "deep/part.stl", "deep/nested/part-holder.stl"]
        );
        assert!(score("box", "box.stl") > score("box", "big-old-x.stl"));
    }
}
