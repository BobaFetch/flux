//! Fuzzy matching for the pickers: a query matches a candidate when its
//! characters appear in order (case-insensitively), scored so tighter,
//! word-aligned matches rank first. (Command-line completion is prefix-based,
//! like Vim's; only the pickers use this.)

/// Characters after which a match counts as word-aligned.
fn is_sep(c: char) -> bool {
    matches!(c, '/' | '_' | '-' | '.' | ' ')
}

/// Align `query` to `cand` (both lowercased): the leftmost match, or the
/// rightmost when `from_left` is false. Neither pass is optimal on its own;
/// [`score`] takes the better of the two.
fn align(query: &[char], cand: &[char], from_left: bool) -> Option<Vec<usize>> {
    let mut positions = Vec::with_capacity(query.len());
    if from_left {
        let mut qi = 0;
        for (ci, &c) in cand.iter().enumerate() {
            if c == query[qi] {
                positions.push(ci);
                qi += 1;
                if qi == query.len() {
                    break;
                }
            }
        }
        if qi != query.len() {
            return None;
        }
    } else {
        let mut qi = query.len();
        for (ci, &c) in cand.iter().enumerate().rev() {
            if qi > 0 && c == query[qi - 1] {
                positions.push(ci);
                qi -= 1;
            }
        }
        if qi != 0 {
            return None;
        }
        positions.reverse();
    }
    Some(positions)
}

/// Score one alignment: +10 per char, +10 word-aligned, +15 contiguous, -4 per
/// skipped char, -1 per candidate char. A word-aligned char across a one-char
/// gap nets +6 bonus, less than the +15 contiguity bonus, so tight matches
/// beat scattered word-start matches while alignment still breaks ties.
fn score_positions(cand: &[char], positions: &[usize]) -> i64 {
    let mut total: i64 = 0;
    let mut prev: Option<usize> = None;
    for &p in positions {
        total += 10;
        if p == 0 || is_sep(cand[p - 1]) {
            total += 10;
        }
        if prev.is_some_and(|q| q + 1 == p) {
            total += 15;
        }
        if let Some(q) = prev {
            total -= 4 * (p - q - 1) as i64;
        }
        prev = Some(p);
    }
    total -= cand.len() as i64;
    total
}

/// Score `query` against `candidate` (higher is better), or `None` when the
/// query isn't a case-insensitive subsequence of the candidate. An empty query
/// matches everything with score 0.
pub fn score(query: &str, candidate: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    let qs: Vec<char> = query.chars().map(|c| c.to_ascii_lowercase()).collect();
    let cs: Vec<char> = candidate.chars().collect();
    let ls: Vec<char> = cs.iter().map(|c| c.to_ascii_lowercase()).collect();
    let left = align(&qs, &ls, true).map(|p| score_positions(&cs, &p));
    let right = align(&qs, &ls, false).map(|p| score_positions(&cs, &p));
    match (left, right) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) | (None, Some(a)) => Some(a),
        (None, None) => None,
    }
}

/// Indices of `candidates` matching `query`, best first (stable: ties keep
/// walk order). An empty query matches everything.
pub fn rank(query: &str, candidates: &[String]) -> Vec<usize> {
    let mut scored: Vec<(usize, i64)> = candidates
        .iter()
        .enumerate()
        .filter_map(|(i, c)| score(query, c).map(|s| (i, s)))
        .collect();
    scored.sort_by_key(|a| std::cmp::Reverse(a.1));
    scored.into_iter().map(|(i, _)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subsequence_and_case() {
        assert!(score("abc", "a_b_c").is_some());
        assert!(score("abc", "acb").is_none());
        assert!(score("EDIT", "editor.rs").is_some());
        assert_eq!(score("", "anything"), Some(0));
        assert_eq!(score("a", ""), None);
    }

    #[test]
    fn tighter_and_aligned_ranks_first() {
        let tight = score("edit", "editor.rs").unwrap();
        let loose = score("edit", "e-x-d-i-t.rs").unwrap();
        assert!(tight > loose, "{tight} > {loose}");
        let after_sep = score("rs", "a.rs").unwrap();
        let mid_word = score("rs", "ours").unwrap();
        assert!(after_sep > mid_word, "{after_sep} > {mid_word}");
        // ... but consecutiveness can still win from mid-word.
        let consec = score("tor", "editor.rs").unwrap();
        let split = score("tor", "t-o-r.rs").unwrap();
        assert!(consec > split, "{consec} > {split}");
    }

    #[test]
    fn rank_orders_and_filters() {
        let cands = ["xeditor.rs", "editor.rs", "zzz", "edit"]
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        assert_eq!(rank("edit", &cands), vec![3, 1, 0]);
        assert_eq!(rank("", &cands), vec![0, 1, 2, 3]);
        assert!(rank("qqq", &cands).is_empty());
    }
}
