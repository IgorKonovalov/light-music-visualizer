//! L-system grammar expansion: pure, deterministic string rewriting. Applies a
//! production rule set to an axiom `depth` times — each character is replaced by
//! its successor (or kept if no rule matches). This is a build-time step (runs
//! inside `Scene::configure`, off the hot path), not per-frame work.
//!
//! Deterministic by construction: a fixed `(axiom, rules, depth)` always yields
//! the exact same string (NFR 6), which is what makes it directly unit-testable.

// Under render/, so it carries the hygiene guard's panic pragma even though it
// runs only at preset load — written allocation-tolerant but panic-free.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

/// Expand `axiom` by applying `rules` `depth` times. Each rule is
/// `(predecessor, successor)`; a character with no matching rule maps to itself
/// (the standard context-free L-system semantics). Pure and deterministic.
///
/// Runs only at preset load. Growth is bounded by the caller clamping `depth`
/// (see `MAX_LSYSTEM_DEPTH`); the turtle then caps the segment count.
pub fn expand(axiom: &str, rules: &[(char, String)], depth: u32) -> String {
    let mut current = axiom.to_string();
    for _ in 0..depth {
        let mut next = String::with_capacity(current.len().saturating_mul(2));
        for ch in current.chars() {
            match rules.iter().find(|(pred, _)| *pred == ch) {
                Some((_, succ)) => next.push_str(succ),
                None => next.push(ch),
            }
        }
        current = next;
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_is_exact_and_deterministic() {
        // Fibonacci-word grammar: trivially hand-verifiable exact strings.
        let rules = [('A', "AB".to_string()), ('B', "A".to_string())];
        assert_eq!(expand("A", &rules, 0), "A");
        assert_eq!(expand("A", &rules, 1), "AB");
        assert_eq!(expand("A", &rules, 2), "ABA");
        assert_eq!(expand("A", &rules, 3), "ABAAB");
        assert_eq!(expand("A", &rules, 5), "ABAABABAABAAB");

        // The plan's named example: Koch edge F -> "F+F--F+F".
        let koch = [('F', "F+F--F+F".to_string())];
        assert_eq!(expand("F", &koch, 1), "F+F--F+F");
        assert_eq!(
            expand("F", &koch, 2),
            "F+F--F+F+F+F--F+F--F+F--F+F+F+F--F+F"
        );

        // Determinism: the same inputs twice are identical.
        assert_eq!(expand("F", &koch, 3), expand("F", &koch, 3));
    }

    #[test]
    fn characters_without_a_rule_pass_through() {
        // `+`, `-`, `[`, `]` have no rules and survive verbatim; only `X`/`F`
        // rewrite. A single expansion of the classic plant axiom.
        let rules = [('X', "F[+X]F".to_string()), ('F', "FF".to_string())];
        assert_eq!(expand("X", &rules, 1), "F[+X]F");
        assert_eq!(expand("X", &rules, 2), "FF[+F[+X]F]FF");
    }
}
