//! `chmod`-style mode specifications.

use crate::{Error, Result};

const USER: u8 = 1;
const GROUP: u8 = 2;
const OTHER: u8 = 4;
const ALL: u8 = USER | GROUP | OTHER;

/// A mode change: octal (`755`) or symbolic (`u+x,go-w`, `a=rX`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModeSpec {
    Absolute(u32),
    Symbolic(Vec<Clause>),
}

/// One comma-separated part of a symbolic mode, e.g. `go-w` or `u+r-x`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clause {
    who: u8,
    actions: Vec<(Op, Perms)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Add,
    Remove,
    Set,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Perms {
    r: bool,
    w: bool,
    x: bool,
    /// `X`: execute only for directories or if someone already has execute.
    cond_x: bool,
    s: bool,
    t: bool,
}

impl ModeSpec {
    pub fn parse(input: &str) -> Result<Self> {
        let invalid = || Error::InvalidArgument(format!("invalid mode `{input}`"));

        if !input.is_empty() && input.len() <= 4 && input.chars().all(|c| matches!(c, '0'..='7')) {
            let mode = u32::from_str_radix(input, 8).map_err(|_| invalid())?;
            return Ok(Self::Absolute(mode));
        }

        let mut clauses = Vec::new();
        for part in input.split(',') {
            let mut chars = part.chars().peekable();
            let mut who = 0;
            while let Some(bit) = chars.peek().and_then(|c| match c {
                'u' => Some(USER),
                'g' => Some(GROUP),
                'o' => Some(OTHER),
                'a' => Some(ALL),
                _ => None,
            }) {
                who |= bit;
                chars.next();
            }
            if who == 0 {
                who = ALL;
            }

            let mut actions = Vec::new();
            while let Some(c) = chars.next() {
                let op = match c {
                    '+' => Op::Add,
                    '-' => Op::Remove,
                    '=' => Op::Set,
                    _ => return Err(invalid()),
                };
                let mut perms = Perms::default();
                while let Some(&c) = chars.peek() {
                    let flag = match c {
                        'r' => &mut perms.r,
                        'w' => &mut perms.w,
                        'x' => &mut perms.x,
                        'X' => &mut perms.cond_x,
                        's' => &mut perms.s,
                        't' => &mut perms.t,
                        _ => break,
                    };
                    *flag = true;
                    chars.next();
                }
                actions.push((op, perms));
            }
            if actions.is_empty() {
                return Err(invalid());
            }
            clauses.push(Clause { who, actions });
        }
        Ok(Self::Symbolic(clauses))
    }

    /// Returns the new permission bits for an entry currently at `mode`.
    pub fn apply(&self, mode: u32, is_dir: bool) -> u32 {
        let clauses = match self {
            Self::Absolute(mode) => return *mode,
            Self::Symbolic(clauses) => clauses,
        };
        let mut mode = mode & 0o7777;
        for clause in clauses {
            for &(op, perms) in &clause.actions {
                let bits = clause.bits(perms, mode, is_dir);
                match op {
                    Op::Add => mode |= bits,
                    Op::Remove => mode &= !bits,
                    Op::Set => mode = (mode & !clause.mask()) | bits,
                }
            }
        }
        mode
    }
}

impl Clause {
    fn bits(&self, p: Perms, mode: u32, is_dir: bool) -> u32 {
        let x = p.x || (p.cond_x && (is_dir || mode & 0o111 != 0));
        let rwx = (p.r as u32) << 2 | (p.w as u32) << 1 | x as u32;
        let mut bits = 0;
        if self.who & USER != 0 {
            bits |= rwx << 6 | if p.s { 0o4000 } else { 0 };
        }
        if self.who & GROUP != 0 {
            bits |= rwx << 3 | if p.s { 0o2000 } else { 0 };
        }
        if self.who & OTHER != 0 {
            bits |= rwx | if p.t { 0o1000 } else { 0 };
        }
        bits
    }

    /// Every bit a `=` assignment for these classes resets.
    fn mask(&self) -> u32 {
        let all = Perms {
            r: true,
            w: true,
            x: true,
            s: true,
            t: true,
            ..Default::default()
        };
        self.bits(all, 0, false)
    }
}

#[cfg(test)]
mod tests {
    use super::ModeSpec;

    fn apply(spec: &str, mode: u32, is_dir: bool) -> u32 {
        ModeSpec::parse(spec).unwrap().apply(mode, is_dir)
    }

    #[test]
    fn octal() {
        assert_eq!(ModeSpec::parse("755").unwrap(), ModeSpec::Absolute(0o755));
        assert_eq!(apply("0640", 0o777, false), 0o640);
    }

    #[test]
    fn symbolic() {
        assert_eq!(apply("u+x", 0o644, false), 0o744);
        assert_eq!(apply("go-w", 0o666, false), 0o644);
        assert_eq!(apply("u=rw,go=r", 0o777, false), 0o644);
        assert_eq!(apply("+x", 0o644, false), 0o755);
        assert_eq!(apply("u+r-w", 0o244, false), 0o444);
        assert_eq!(apply("u+s,+t", 0o755, true), 0o5755);
    }

    #[test]
    fn conditional_execute() {
        assert_eq!(apply("a=rX", 0o700, true), 0o555);
        assert_eq!(apply("a=rX", 0o600, false), 0o444);
        assert_eq!(apply("a=rX", 0o700, false), 0o555);
    }

    #[test]
    fn invalid() {
        for spec in ["", "u", "u+q", "9", "77777", "u+r,", "x+r"] {
            assert!(ModeSpec::parse(spec).is_err(), "{spec} should be invalid");
        }
    }
}
