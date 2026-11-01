//! Text formatting for interpolated strings, done natively (spec: Text output).

use bidule_bytecode::image::{ArgType, Format};

/// A fixed-capacity byte buffer; writes past the end are dropped.
pub struct Buf<'b> {
    pub bytes: &'b mut [u8],
    pub len: usize,
}

impl<'b> Buf<'b> {
    pub fn new(bytes: &'b mut [u8]) -> Self {
        Buf { bytes, len: 0 }
    }
    pub fn push(&mut self, c: u8) {
        if self.len < self.bytes.len() {
            self.bytes[self.len] = c;
            self.len += 1;
        }
    }
    pub fn extend(&mut self, s: &[u8]) {
        for &c in s {
            self.push(c);
        }
    }
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

/// Small scratch for one formatted number.
struct Num {
    b: [u8; 48],
    n: usize,
}

impl Num {
    fn new() -> Self {
        Num { b: [0; 48], n: 0 }
    }
    fn push(&mut self, c: u8) {
        if self.n < self.b.len() {
            self.b[self.n] = c;
            self.n += 1;
        }
    }
    fn digits(&mut self, mut v: u64, min: usize) {
        let mut tmp = [0u8; 20];
        let mut k = 0;
        loop {
            tmp[k] = b'0' + (v % 10) as u8;
            k += 1;
            v /= 10;
            if v == 0 && k >= min {
                break;
            }
        }
        while k > 0 {
            k -= 1;
            self.push(tmp[k]);
        }
    }
    fn s(&self) -> &[u8] {
        &self.b[..self.n]
    }
}

const POW10: [u64; 9] = [1, 10, 100, 1000, 10_000, 100_000, 1_000_000, 10_000_000, 100_000_000];

/// `x.toFixed(d)` for a 16.16 value: round half up on the magnitude, sign kept.
fn fixed(out: &mut Num, raw: i32, d: u8) {
    let neg = raw < 0;
    let m = (raw as i64).unsigned_abs();
    let p = POW10[d as usize];
    let scaled = (m * p * 2 + 65536) / 131072;
    if neg {
        out.push(b'-');
    }
    out.digits(scaled / p, 1);
    if d > 0 {
        out.push(b'.');
        out.digits(scaled % p, d as usize);
    }
}

/// Formats one value the way the browser prototype does (spec: Text output › Interpolation).
pub fn format_value(out: &mut Buf, v: i32, f: &Format) {
    let ty = f.ty;
    if f.chr {
        let c = if ty == ArgType::Fix { v >> 16 } else { v };
        out.push((c & 255) as u8);
        return;
    }
    let mut s = Num::new();
    if f.hex {
        let mut x = v as u32;
        let mut tmp = [0u8; 8];
        let mut k = 0;
        loop {
            let d = (x & 15) as u8;
            tmp[k] = if d < 10 { b'0' + d } else { b'a' + d - 10 };
            k += 1;
            x >>= 4;
            if x == 0 {
                break;
            }
        }
        while k > 0 {
            k -= 1;
            s.push(tmp[k]);
        }
    } else if ty == ArgType::Fix {
        match f.decimals {
            Some(d) => fixed(&mut s, v, d),
            None => {
                fixed(&mut s, v, 4);
                // drop trailing zeros, then a trailing point
                while s.n > 0 && s.b[s.n - 1] == b'0' && s.s().contains(&b'.') {
                    s.n -= 1;
                }
                if s.n > 0 && s.b[s.n - 1] == b'.' {
                    s.n -= 1;
                }
            }
        }
        if s.s() == b"-0" {
            s.n = 0;
            s.push(b'0');
        }
    } else {
        if v < 0 {
            s.push(b'-');
        }
        s.digits((v as i64).unsigned_abs(), 1);
        if let Some(d) = f.decimals {
            if d > 0 {
                s.push(b'.');
                for _ in 0..d {
                    s.push(b'0');
                }
            }
        }
    }
    let w = f.width as usize;
    let body = s.s();
    if body.len() < w {
        let pad = w - body.len();
        if f.zero {
            let (sign, rest) = if body.first() == Some(&b'-') {
                (&body[..1], &body[1..])
            } else {
                (&body[..0], body)
            };
            out.extend(sign);
            for _ in 0..pad {
                out.push(b'0');
            }
            out.extend(rest);
            return;
        }
        for _ in 0..pad {
            out.push(b' ');
        }
    }
    out.extend(body);
}

/// Width in pixels of formatted text: the widest line, 6 pixels per character.
pub fn width(s: &[u8]) -> i32 {
    let mut best = 0;
    for line in s.split(|&c| c == b'\n') {
        best = best.max(line.len() as i32);
    }
    best * 6
}

#[cfg(test)]
mod tests {
    use super::*;
    fn f(ty: ArgType) -> Format {
        Format {
            ty,
            zero: false,
            hex: false,
            chr: false,
            width: 0,
            decimals: None,
        }
    }
    fn run(v: i32, fm: Format) -> [u8; 32] {
        let mut b = [0u8; 32];
        let mut buf = Buf::new(&mut b[..]);
        format_value(&mut buf, v, &fm);
        let n = buf.len;
        let mut out = [0u8; 32];
        out[..n].copy_from_slice(&b[..n]);
        out
    }
    fn s(v: i32, fm: Format) -> &'static str {
        let o = run(v, fm);
        let n = o.iter().position(|&c| c == 0).unwrap_or(32);
        alloc_str(&o[..n])
    }
    fn alloc_str(b: &[u8]) -> &'static str {
        // tests only: leak a copy
        extern crate std;
        std::boxed::Box::leak(std::string::String::from_utf8(b.to_vec()).unwrap().into_boxed_str())
    }
    #[test]
    fn ints() {
        assert_eq!(
            s(
                120,
                Format {
                    zero: true,
                    width: 4,
                    ..f(ArgType::Int)
                }
            ),
            "0120"
        );
        assert_eq!(
            s(
                -5,
                Format {
                    zero: true,
                    width: 4,
                    ..f(ArgType::Int)
                }
            ),
            "-005"
        );
        assert_eq!(
            s(
                42,
                Format {
                    width: 5,
                    ..f(ArgType::Int)
                }
            ),
            "   42"
        );
        assert_eq!(
            s(
                255,
                Format {
                    hex: true,
                    ..f(ArgType::Int)
                }
            ),
            "ff"
        );
        assert_eq!(
            s(
                -1,
                Format {
                    hex: true,
                    ..f(ArgType::Int)
                }
            ),
            "ffffffff"
        );
        assert_eq!(
            s(
                65,
                Format {
                    chr: true,
                    ..f(ArgType::Int)
                }
            ),
            "A"
        );
        assert_eq!(
            s(
                5,
                Format {
                    decimals: Some(2),
                    ..f(ArgType::Int)
                }
            ),
            "5.00"
        );
        assert_eq!(s(i32::MIN, f(ArgType::Int)), "-2147483648");
    }
    #[test]
    fn fixes() {
        assert_eq!(s(98304, f(ArgType::Fix)), "1.5");
        assert_eq!(s(65536, f(ArgType::Fix)), "1");
        assert_eq!(s(3277, f(ArgType::Fix)), "0.05");
        assert_eq!(s(-1, f(ArgType::Fix)), "0");
        assert_eq!(s(-98304, f(ArgType::Fix)), "-1.5");
        assert_eq!(
            s(
                98304,
                Format {
                    decimals: Some(1),
                    ..f(ArgType::Fix)
                }
            ),
            "1.5"
        );
        assert_eq!(
            s(
                6554,
                Format {
                    decimals: Some(1),
                    ..f(ArgType::Fix)
                }
            ),
            "0.1"
        );
        assert_eq!(
            s(
                -1,
                Format {
                    decimals: Some(2),
                    ..f(ArgType::Fix)
                }
            ),
            "-0.00"
        );
        assert_eq!(
            s(
                98304,
                Format {
                    width: 6,
                    decimals: Some(2),
                    ..f(ArgType::Fix)
                }
            ),
            "  1.50"
        );
    }
}
