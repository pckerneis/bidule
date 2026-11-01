//! 8-bit indexed framebuffer, 256-entry RGB565 palette and drawing primitives
//! (spec: Display). Same pixels on every target.

use crate::font::{CH, CW, GLYPHS};
use bidule_bytecode::asset::{SHEET_COLS, TILE_COUNT};

pub const W: i32 = 240;
pub const H: i32 = 160;

/// Default palette entries 0–15 (mimi16, after DawnBringer's DB16), RGB565.
pub const PAL_DEFAULT: [u16; 16] = [
    0x1063, 0x31AD, 0x4126, 0x3324, 0x8266, 0x4A49, 0x9D36, 0xEF9C, 0xCA29, 0xD3E5, 0xDE8B, 0x6D45, 0x5BF9, 0x6E19, 0xA88E, 0xBD4B,
];

pub struct Screen {
    pub fb: [u8; (W * H) as usize],
    pub pal: [u16; 256],
    /// Palette index skipped when drawing tiles and sprites (`palt`).
    pub transparent: u8,
}

impl Default for Screen {
    fn default() -> Self {
        let mut s = Screen {
            fb: [0; (W * H) as usize],
            pal: [0; 256],
            transparent: 0,
        };
        s.reset_palette();
        s
    }
}

/// Expands RGB565 to 8-bit components.
pub fn rgb888(w: u16) -> [u8; 3] {
    let r = (w >> 11 & 31) as u8;
    let g = (w >> 5 & 63) as u8;
    let b = (w & 31) as u8;
    [(r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2)]
}

impl Screen {
    pub fn reset_palette(&mut self) {
        self.pal = [0; 256];
        self.pal[..16].copy_from_slice(&PAL_DEFAULT);
    }

    /// `setpal(i, r, g, b)`: 8-bit components rounded to RGB565.
    pub fn setpal(&mut self, i: u8, r: i32, g: i32, b: i32) {
        let c = |v: i32, m: i32| ((v.clamp(0, 255) * m * 2 + 255) / 510) as u16;
        self.pal[i as usize] = c(r, 31) << 11 | c(g, 63) << 5 | c(b, 31);
    }

    pub fn getpal(&self, i: u8, chan: usize) -> i32 {
        rgb888(self.pal[i as usize])[chan.min(2)] as i32
    }

    pub fn cls(&mut self, c: u8) {
        self.fb.fill(c);
    }

    #[inline]
    pub fn px(&mut self, x: i32, y: i32, c: u8) {
        if x >= 0 && y >= 0 && x < W && y < H {
            self.fb[(y * W + x) as usize] = c;
        }
    }

    pub fn pget(&self, x: i32, y: i32) -> u8 {
        if x >= 0 && y >= 0 && x < W && y < H {
            self.fb[(y * W + x) as usize]
        } else {
            0
        }
    }

    fn hspan(&mut self, mut x0: i32, mut x1: i32, y: i32, c: u8) {
        if !(0..H).contains(&y) {
            return;
        }
        if x0 > x1 {
            core::mem::swap(&mut x0, &mut x1);
        }
        let x0 = x0.max(0);
        let x1 = x1.min(W - 1);
        if x0 <= x1 {
            self.fb[(y * W + x0) as usize..=(y * W + x1) as usize].fill(c);
        }
    }

    pub fn rectfill(&mut self, x: i32, y: i32, w: i32, h: i32, c: u8) {
        // rows outside the screen draw nothing; bound the loop for huge sizes
        let y0 = y.max(-1);
        let y1 = y.saturating_add(h).min(H + 1);
        for yy in y0..y1 {
            self.hspan(x, x.saturating_add(w) - 1, yy, c);
        }
    }

    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: u8) {
        if w <= 0 || h <= 0 {
            return;
        }
        let x1 = x.saturating_add(w) - 1;
        let y1 = y.saturating_add(h) - 1;
        self.hspan(x, x1, y, c);
        self.hspan(x, x1, y1, c);
        for yy in (y + 1).max(-1)..y1.min(H + 1) {
            self.px(x, yy, c);
            self.px(x1, yy, c);
        }
    }

    /// Bresenham line, both ends included (at most 4096 points).
    pub fn line(&mut self, mut x0: i32, mut y0: i32, x1: i32, y1: i32, c: u8) {
        let dx = (x1 as i64 - x0 as i64).abs();
        let dy = -(y1 as i64 - y0 as i64).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        for _ in 0..4096 {
            self.px(x0, y0, c);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 = x0.wrapping_add(sx);
            }
            if e2 <= dx {
                err += dx;
                y0 = y0.wrapping_add(sy);
            }
        }
    }

    /// Midpoint circle, outline or filled.
    pub fn circle(&mut self, cx: i32, cy: i32, r: i32, c: u8, fill: bool) {
        if r < 0 {
            return;
        }
        // nothing of a circle this far away can reach the screen
        if r > 4 * W || cx.unsigned_abs() > 8 * W as u32 || cy.unsigned_abs() > 8 * W as u32 {
            return;
        }
        let (mut x, mut y, mut e) = (r, 0, 1 - r);
        while x >= y {
            if fill {
                self.hspan(cx - x, cx + x, cy + y, c);
                self.hspan(cx - x, cx + x, cy - y, c);
                self.hspan(cx - y, cx + y, cy + x, c);
                self.hspan(cx - y, cx + y, cy - x, c);
            } else {
                for (a, b) in [(x, y), (y, x), (-y, x), (-x, y), (-x, -y), (-y, -x), (y, -x), (x, -y)] {
                    self.px(cx + a, cy + b, c);
                }
            }
            y += 1;
            if e < 0 {
                e += 2 * y + 1;
            } else {
                x -= 1;
                e += 2 * (y - x) + 1;
            }
        }
    }

    /// Draws text; returns the x position after the last character.
    pub fn text(&mut self, s: &[u8], x: i32, y: i32, c: u8) -> i32 {
        let x0 = x;
        let (mut x, mut y) = (x, y);
        for &ch in s {
            if ch == b'\n' {
                x = x0;
                y = y.saturating_add(CH);
                continue;
            }
            let g = (if (32..=126).contains(&ch) { ch } else { b'?' } - 32) as usize;
            for r in 0..CH {
                let bits = GLYPHS[g * 9 + r as usize];
                if bits == 0 {
                    continue;
                }
                for b in 0..5 {
                    if bits & (1 << b) != 0 {
                        self.px(x.saturating_add(b), y.saturating_add(r), c);
                    }
                }
            }
            x = x.saturating_add(CW);
        }
        x
    }

    /// One 8×8 tile of a sheet, skipping the transparent index.
    pub fn tile(&mut self, sheet: &[u8], t: i32, x: i32, y: i32) {
        if t < 0 || t >= TILE_COUNT as i32 || x <= -8 || y <= -8 || x >= W || y >= H {
            return;
        }
        let base = t as usize * 64;
        if base + 64 > sheet.len() {
            return;
        }
        for j in 0..8 {
            let yy = y + j;
            if !(0..H).contains(&yy) {
                continue;
            }
            for i in 0..8 {
                let c = sheet[base + (j * 8 + i) as usize];
                let xx = x + i;
                if c != self.transparent && (0..W).contains(&xx) {
                    self.fb[(yy * W + xx) as usize] = c;
                }
            }
        }
    }

    /// A sprite of `w`×`h` tiles starting at `tile`.
    pub fn sprite(&mut self, sheet: &[u8], tile: i32, w: i32, h: i32, x: i32, y: i32) {
        for ty in 0..h.min(SHEET_COLS as i32) {
            for tx in 0..w.min(SHEET_COLS as i32) {
                self.tile(
                    sheet,
                    tile + ty * SHEET_COLS as i32 + tx,
                    x.saturating_add(tx * 8),
                    y.saturating_add(ty * 8),
                );
            }
        }
    }
}
