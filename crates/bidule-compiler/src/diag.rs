//! Diagnostics: errors and warnings with a source position.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pos {
    pub line: u32,
    pub col: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diag {
    pub severity: Severity,
    pub pos: Pos,
    pub msg: String,
}

impl Diag {
    pub fn error(pos: Pos, msg: impl Into<String>) -> Diag {
        Diag {
            severity: Severity::Error,
            pos,
            msg: msg.into(),
        }
    }
    pub fn warning(pos: Pos, msg: impl Into<String>) -> Diag {
        Diag {
            severity: Severity::Warning,
            pos,
            msg: msg.into(),
        }
    }
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Diag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{}:{}: {}: {}", self.pos.line, self.pos.col, s, self.msg)
    }
}

/// Edit distance, for "did you mean" suggestions.
pub fn distance(a: &str, b: &str) -> usize {
    let b = b.as_bytes();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.bytes().enumerate() {
        let mut cur = Vec::with_capacity(b.len() + 1);
        cur.push(i + 1);
        for (j, &cb) in b.iter().enumerate() {
            let sub = prev[j] + (ca != cb) as usize;
            cur.push(sub.min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// The closest candidate within a small edit distance.
pub fn suggest<'a>(name: &str, candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let limit = if name.len() <= 3 { 1 } else { 2 };
    candidates
        .map(|c| (distance(name, c), c))
        .filter(|(d, _)| *d <= limit)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}
