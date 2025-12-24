use super::*;

impl FathomApp {
    fn normalize_for_search(s: &str) -> String {
        s.to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { ' ' })
            .collect()
    }

    fn is_subsequence(needle: &str, haystack: &str) -> bool {
        if needle.is_empty() {
            return true;
        }
        let mut it = haystack.chars();
        for n in needle.chars() {
            loop {
                match it.next() {
                    Some(h) if h == n => break,
                    Some(_) => continue,
                    None => return false,
                }
            }
        }
        true
    }

    fn elastic_match(haystack: &str, term: &str) -> bool {
        if term.is_empty() {
            return true;
        }

        // Fast path: substring match.
        if haystack.contains(term) {
            return true;
        }

        // Fuzzy path: subsequence match for “elastic” feel.
        Self::is_subsequence(term, haystack)
    }

    pub(super) fn pod_matches_search(&self, pod: &PodSummary, query: &str) -> bool {
        let q = query.trim();
        if q.is_empty() {
            return true;
        }

        let haystack_raw = format!(
            "{} {} {} {} {} {}/{} {}",
            pod.namespace,
            pod.name,
            pod.phase,
            pod.node,
            pod.restarts,
            pod.ready,
            pod.total,
            // duplicate combined path-ish token
            format!("{}/{}", pod.namespace, pod.name)
        );
        let haystack = Self::normalize_for_search(&haystack_raw);

        for term in Self::normalize_for_search(q).split_whitespace() {
            if !Self::elastic_match(&haystack, term) {
                return false;
            }
        }
        true
    }
}
