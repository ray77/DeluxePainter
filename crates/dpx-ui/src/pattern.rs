//! AmigaDOS-style file name patterns for the file requester's "Pattern" field.
//!
//! The matcher is dependency-free and works on `char`s. Matching is anchored
//! at both ends and case-insensitive (simple lowercase folding). It simulates
//! the pattern as a set of reachable name positions, so matching time is
//! polynomial in pattern and name length (no exponential backtracking).
//!
//! Supported syntax:
//!
//! - empty pattern: matches every name
//! - `?`: any single character
//! - `#x`: zero or more repetitions of `x`, where `x` is one item: a literal,
//!   `?`, a `[...]` class or a `(...)` group (`#?` matches anything)
//! - `*`: synonym of `#?` (a DeluxePainter addition, like AmigaOS' wildstar
//!   option; it is not part of the classic pattern syntax)
//! - `[abc]`, `[a-z]`: character class; `[~abc]` negates the class
//! - `(a|b|c)`: alternatives; each alternative is a full pattern sequence and
//!   may be empty or contain nested groups
//! - `~`: negation, supported only as the first character of the whole
//!   pattern or of an alternative/group body. It negates the rest of that
//!   sequence: `~(#?.info)` matches every name not ending in `.info`, and
//!   `(a|~b#?)` is "a, or anything not starting with b". A `~` anywhere else
//!   (mid-sequence, or as the operand of `#`) is an ordinary literal.
//! - `%`: matches the empty string
//! - `'x`: escapes the next character, so `'?` is a literal question mark
//! - `|` outside parentheses is an ordinary literal; any other character
//!   matches itself.
//!
//! Errors: `#` or `'` at the end of the pattern, unbalanced parentheses,
//! an unterminated `[`, and nesting deeper than 64 levels.

use std::fmt;

const MAX_DEPTH: usize = 64;

/// A parsed AmigaDOS file name pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    root: Seq,
}

/// Error returned by [`Pattern::parse`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatternError {
    pub message: String,
}

impl fmt::Display for PatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid pattern: {}", self.message)
    }
}

impl std::error::Error for PatternError {}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Seq {
    negated: bool,
    nodes: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    Any,
    Empty,
    /// Literal, stored lowercase-folded.
    Lit(char),
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
    Star(Box<Node>),
    Group(Vec<Seq>),
}

fn err<T>(message: &str) -> Result<T, PatternError> {
    Err(PatternError {
        message: message.to_string(),
    })
}

fn lower(c: char) -> char {
    let mut it = c.to_lowercase();
    match (it.next(), it.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

fn upper(c: char) -> char {
    let mut it = c.to_uppercase();
    match (it.next(), it.next()) {
        (Some(u), None) => u,
        _ => c,
    }
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    /// Parses a sequence until end of input, `|` (only in groups) or `)`.
    fn seq(&mut self, depth: usize, in_group: bool) -> Result<Seq, PatternError> {
        if depth > MAX_DEPTH {
            return err("nesting too deep");
        }
        let mut negated = false;
        if self.peek() == Some('~') {
            negated = true;
            self.pos += 1;
        }
        let mut nodes = Vec::new();
        while let Some(c) = self.peek() {
            if c == ')' || (c == '|' && in_group) {
                break;
            }
            nodes.push(self.item(depth)?);
        }
        Ok(Seq { negated, nodes })
    }

    fn item(&mut self, depth: usize) -> Result<Node, PatternError> {
        let Some(c) = self.peek() else {
            return err("unexpected end of pattern");
        };
        self.pos += 1;
        match c {
            '?' => Ok(Node::Any),
            '*' => Ok(Node::Star(Box::new(Node::Any))),
            '%' => Ok(Node::Empty),
            '#' => match self.peek() {
                None => err("'#' at end of pattern"),
                Some(')') => err("'#' without operand"),
                Some('|') if depth > 0 => err("'#' without operand"),
                Some(_) => {
                    if depth + 1 > MAX_DEPTH {
                        return err("nesting too deep");
                    }
                    let inner = self.item(depth + 1)?;
                    Ok(Node::Star(Box::new(inner)))
                }
            },
            '\'' => match self.peek() {
                None => err("'''' at end of pattern"),
                Some(l) => {
                    self.pos += 1;
                    Ok(Node::Lit(lower(l)))
                }
            },
            '[' => self.class(),
            '(' => {
                let mut alts = Vec::new();
                loop {
                    alts.push(self.seq(depth + 1, true)?);
                    match self.peek() {
                        Some('|') => self.pos += 1,
                        Some(')') => {
                            self.pos += 1;
                            break;
                        }
                        _ => return err("unbalanced '('"),
                    }
                }
                Ok(Node::Group(alts))
            }
            ')' => err("unbalanced ')'"),
            ']' => err("unmatched ']'"),
            other => Ok(Node::Lit(lower(other))),
        }
    }

    fn class(&mut self) -> Result<Node, PatternError> {
        let mut negated = false;
        if self.peek() == Some('~') {
            negated = true;
            self.pos += 1;
        }
        let mut ranges = Vec::new();
        loop {
            let Some(c) = self.peek() else {
                return err("unterminated '['");
            };
            self.pos += 1;
            if c == ']' {
                break;
            }
            let lo = if c == '\'' {
                let Some(e) = self.peek() else {
                    return err("unterminated '['");
                };
                self.pos += 1;
                e
            } else {
                c
            };
            if self.peek() == Some('-') && self.chars.get(self.pos + 1).is_some_and(|&n| n != ']') {
                self.pos += 1;
                let mut hi = self.peek().unwrap_or(lo);
                self.pos += 1;
                if hi == '\'' {
                    let Some(e) = self.peek() else {
                        return err("unterminated '['");
                    };
                    self.pos += 1;
                    hi = e;
                }
                ranges.push(if lo <= hi { (lo, hi) } else { (hi, lo) });
            } else {
                ranges.push((lo, lo));
            }
        }
        Ok(Node::Class { negated, ranges })
    }
}

type Set = Vec<bool>;

fn step(node: &Node, name: &[char], set: &Set) -> Set {
    let n = name.len();
    let mut out = vec![false; n + 1];
    match node {
        Node::Empty => out.clone_from(set),
        Node::Any => {
            for i in 0..n {
                out[i + 1] |= set[i];
            }
        }
        Node::Lit(l) => {
            for i in 0..n {
                if set[i] && lower(name[i]) == *l {
                    out[i + 1] = true;
                }
            }
        }
        Node::Class { negated, ranges } => {
            for i in 0..n {
                if set[i] {
                    let c = name[i];
                    let (lc, uc) = (lower(c), upper(c));
                    let hit = ranges
                        .iter()
                        .any(|&(lo, hi)| [c, lc, uc].iter().any(|x| (lo..=hi).contains(x)));
                    if hit != *negated {
                        out[i + 1] = true;
                    }
                }
            }
        }
        Node::Star(inner) => {
            let mut acc = set.clone();
            let mut frontier = set.clone();
            loop {
                let next = step(inner, name, &frontier);
                let mut fresh = vec![false; n + 1];
                let mut any = false;
                for i in 0..=n {
                    if next[i] && !acc[i] {
                        fresh[i] = true;
                        acc[i] = true;
                        any = true;
                    }
                }
                if !any {
                    break;
                }
                frontier = fresh;
            }
            out = acc;
        }
        Node::Group(alts) => {
            for alt in alts {
                let r = run_seq(alt, name, set);
                for i in 0..=n {
                    out[i] |= r[i];
                }
            }
        }
    }
    out
}

fn run_seq(seq: &Seq, name: &[char], set: &Set) -> Set {
    let n = name.len();
    if !seq.negated {
        return run_nodes(&seq.nodes, name, set.clone());
    }
    // Negation: from each start, every end position the sequence cannot reach.
    let mut out = vec![false; n + 1];
    for s in 0..=n {
        if !set[s] {
            continue;
        }
        let mut one = vec![false; n + 1];
        one[s] = true;
        let ends = run_nodes(&seq.nodes, name, one);
        for e in s..=n {
            if !ends[e] {
                out[e] = true;
            }
        }
    }
    out
}

fn run_nodes(nodes: &[Node], name: &[char], mut set: Set) -> Set {
    for node in nodes {
        if !set.iter().any(|&b| b) {
            break;
        }
        set = step(node, name, &set);
    }
    set
}

impl Pattern {
    /// Parses a pattern.
    pub fn parse(src: &str) -> Result<Pattern, PatternError> {
        let mut p = Parser {
            chars: src.chars().collect(),
            pos: 0,
        };
        let root = p.seq(0, false)?;
        if p.pos < p.chars.len() {
            // Only an unmatched ')' stops the top-level sequence early.
            return err("unbalanced ')'");
        }
        Ok(Pattern { root })
    }

    /// Case-insensitive (ASCII and Unicode simple lowercase) like the Amiga file system.
    pub fn matches(&self, name: &str) -> bool {
        if !self.root.negated && self.root.nodes.is_empty() {
            return true; // empty pattern matches everything
        }
        let chars: Vec<char> = name.chars().collect();
        let mut start = vec![false; chars.len() + 1];
        start[0] = true;
        run_seq(&self.root, &chars, &start)[chars.len()]
    }

    /// True for an empty pattern or one that matches every name (`#?`, `*`).
    pub fn is_match_all(&self) -> bool {
        !self.root.negated
            && self
                .root
                .nodes
                .iter()
                .all(|n| matches!(n, Node::Empty) || *n == Node::Star(Box::new(Node::Any)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pat: &str, name: &str) -> bool {
        Pattern::parse(pat).unwrap().matches(name)
    }

    #[test]
    fn empty_matches_everything() {
        assert!(m("", ""));
        assert!(m("", "anything.iff"));
    }

    #[test]
    fn literals_are_anchored() {
        assert!(m("abc", "abc"));
        assert!(!m("abc", "abcd"));
        assert!(!m("abc", "xabc"));
        assert!(!m("abc", ""));
    }

    #[test]
    fn question_mark() {
        assert!(m("a?c", "abc"));
        assert!(m("a?c", "aäc"));
        assert!(!m("a?c", "ac"));
        assert!(!m("a?c", "abbc"));
    }

    #[test]
    fn hash_repetition() {
        assert!(m("#?", ""));
        assert!(m("#?.iff", "x.iff"));
        assert!(m("#?.iff", ".iff"));
        assert!(!m("#?.iff", "x.iffx"));
        assert!(m("a#b", "a"));
        assert!(m("a#b", "abbb"));
        assert!(!m("a#b", "abc"));
        assert!(m("#(ab)", "ababab"));
        assert!(!m("#(ab)", "aba"));
        assert!(m("#[0-9]", "2024"));
        assert!(!m("#[0-9]", "20a4"));
    }

    #[test]
    fn star_synonym() {
        assert!(m("*", "whatever"));
        assert!(m("*.iff", "pic.iff"));
        assert!(m("a*b*c", "aXXbYYc"));
        assert!(!m("a*b*c", "aXXbYY"));
    }

    #[test]
    fn case_insensitive() {
        assert!(m("#?.IFF", "pic.iff"));
        assert!(m("#?.iff", "PIC.IFF"));
        assert!(m("[a-c]x", "Bx"));
        assert!(m("[A-C]x", "bx"));
        assert!(m("ÄÖ", "äö"));
        assert!(!m("[~a-c]x", "Bx"));
    }

    #[test]
    fn classes() {
        assert!(m("[abc]", "b"));
        assert!(!m("[abc]", "d"));
        assert!(m("[a-z0-9]#?", "q123"));
        assert!(!m("[a-z]", "5"));
        assert!(m("[~0-9]", "x"));
        assert!(!m("[~0-9]", "5"));
        assert!(m("[a-]", "-"));
        assert!(m("[z-a]", "m"));
        assert!(m("['])]", ")"));
        assert!(!m("[]", "a"));
    }

    #[test]
    fn alternatives() {
        assert!(m("(a|b|c)", "b"));
        assert!(!m("(a|b|c)", "d"));
        assert!(m("#?.(iff|png|ilbm)", "x.PNG"));
        assert!(!m("#?.(iff|png)", "x.jpg"));
        assert!(m("x(|y)z", "xz"));
        assert!(m("x(|y)z", "xyz"));
        assert!(m("(a(b|c)d|e)", "acd"));
        assert!(m("(a(b|c)d|e)", "e"));
        assert!(!m("(a(b|c)d|e)", "ad"));
        assert!(m("a|b", "a|b"));
    }

    #[test]
    fn negation() {
        assert!(m("~(#?.info)", "pic.iff"));
        assert!(!m("~(#?.info)", "pic.info"));
        assert!(m("~(#?.info)", ""));
        assert!(m("~abc", "abd"));
        assert!(!m("~abc", "abc"));
        assert!(m("(a|~b#?)", "a"));
        assert!(m("(a|~b#?)", "cat"));
        assert!(!m("(a|~b#?)", "bat"));
        assert!(m("x(~y)", "xz"));
        assert!(!m("x(~y)", "xy"));
        // Not at a sequence start: literal tilde.
        assert!(m("a~b", "a~b"));
        assert!(!m("a~b", "ab"));
    }

    #[test]
    fn percent_is_empty() {
        assert!(m("%", ""));
        assert!(!m("%", "a"));
        assert!(m("a%b", "ab"));
        assert!(m("(%|x)y", "y"));
    }

    #[test]
    fn escapes() {
        assert!(m("'?", "?"));
        assert!(!m("'?", "a"));
        assert!(m("a'#b", "a#b"));
        assert!(m("'*.txt", "*.txt"));
        assert!(!m("'*.txt", "x.txt"));
        assert!(m("''", "'"));
        assert!(m("'(x'|y')", "(x|y)"));
    }

    #[test]
    fn errors() {
        for bad in [
            "#", "a#", "abc'", "(a", "a)", "(a|b", "[abc", "a]", "(#)", "(a|#)", "#)",
        ] {
            let e = Pattern::parse(bad).unwrap_err();
            assert!(!e.message.is_empty(), "{bad}");
            assert!(e.to_string().contains(&e.message));
        }
        let deep = "(".repeat(200) + &")".repeat(200);
        assert!(Pattern::parse(&deep).is_err());
    }

    #[test]
    fn is_match_all() {
        assert!(Pattern::parse("").unwrap().is_match_all());
        assert!(Pattern::parse("#?").unwrap().is_match_all());
        assert!(Pattern::parse("*").unwrap().is_match_all());
        assert!(!Pattern::parse("*.iff").unwrap().is_match_all());
        assert!(!Pattern::parse("~(#?)").unwrap().is_match_all());
        assert!(!Pattern::parse("?").unwrap().is_match_all());
    }

    #[test]
    fn no_exponential_blowup() {
        let p = Pattern::parse("#?#?#?#?#?x").unwrap();
        let name = "a".repeat(60);
        let t = std::time::Instant::now();
        assert!(!p.matches(&name));
        assert!(p.matches(&(name + "x")));
        let q = Pattern::parse("#(#(#(a|aa)))b").unwrap();
        assert!(!q.matches(&"a".repeat(60)));
        assert!(t.elapsed().as_secs() < 5);
    }

    #[test]
    fn never_panics_on_random_input() {
        let alphabet: Vec<char> = "#?*[]()|~%'-aAbz.".chars().collect();
        let mut state: u64 = 0x1234_5678_9abc_def0;
        let mut next = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as usize
        };
        for _ in 0..600 {
            let len = next() % 14;
            let src: String = (0..len)
                .map(|_| alphabet[next() % alphabet.len()])
                .collect();
            if let Ok(p) = Pattern::parse(&src) {
                for name in ["", "a", "ab.iff", "A.b-z", "(a|b)", "~", "zzzz.a"] {
                    let _ = p.matches(name);
                }
                let _ = p.is_match_all();
            }
        }
    }
}
