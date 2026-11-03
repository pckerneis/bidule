//! The system side of the cart API: drawing, input, sound requests and asset
//! access for a running cart (the VM's [`Host`]).
//!
//! Platforms (browser, desktop, handheld, hardware) own the display, the audio
//! output and the system dialogs; they read the framebuffer, drain sound
//! requests and answer dialogs through this type.

use bidule_bytecode::asset::{AssetSection, SHEET_BYTES, TILE_COUNT};
use bidule_bytecode::builtins::id;
use bidule_bytecode::image::{section, Cart};
use bidule_vm::{sprite_decl, Call, Host, Mem};

use crate::gfx::{Screen, H};

/// A sound request for the platform's mixer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundEvent {
    Play { voice: u8, slot: u16 },
    Stop { voice: u8 },
}

/// A system dialog the cart is waiting on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialog {
    /// `pickcart(kind)`: answer with a handle, or -1 if cancelled.
    PickCart,
    /// `savecart(h)`: answer 1 if saved, 0 if not.
    SaveCart { handle: i32 },
    /// `requestwrite(h)`: answer 1 if the user lets the cart modify the other cart, 0 if not.
    RequestWrite { handle: i32 },
}

const STATUS_MAX: usize = 40;
const EVENTS: usize = 16;

pub struct Console<'c> {
    pub screen: Screen,
    sprites: Option<AssetSection<'c>>,
    maps: Option<AssetSection<'c>>,
    sounds: Option<AssetSection<'c>>,
    /// Held buttons this frame and last frame (bit i = button i).
    pub input: u32,
    prev_input: u32,
    /// Pointer (x, y, buttons) where a mouse exists.
    pub pointer: Option<(i32, i32, i32)>,
    status: [u8; STATUS_MAX],
    status_len: usize,
    /// Milliseconds since the cart started; the platform advances it.
    pub now_ms: u32,
    voices: [(u32, u32); 4],
    pub cues: [i32; 8],
    events: [Option<SoundEvent>; EVENTS],
    pub dialog: Option<Dialog>,
}

impl<'c> Console<'c> {
    /// A console for a cart image whose container is already checked.
    pub fn new(cart: &Cart<'c>) -> Self {
        let sec = |k| cart.section(k).and_then(|b| AssetSection::parse(b).ok());
        let mut screen = Screen::default();
        if let Some(p) = sec(section::PALETTE) {
            if p.payload.len() >= 512 {
                for i in 0..256 {
                    screen.pal[i] = u16::from_le_bytes([p.payload[i * 2], p.payload[i * 2 + 1]]);
                }
            }
        }
        Console {
            screen,
            sprites: sec(section::SPRITES),
            maps: sec(section::MAPS),
            sounds: sec(section::SOUNDS),
            input: 0,
            prev_input: 0,
            pointer: None,
            status: [0; STATUS_MAX],
            status_len: 0,
            now_ms: 0,
            voices: [(0, 0); 4],
            cues: [0; 8],
            events: [None; EVENTS],
            dialog: None,
        }
    }

    /// Call once per frame before `update`, with the buttons held now.
    pub fn begin_frame(&mut self, input: u32) {
        self.prev_input = self.input;
        // Bits 6 and 7 (X and Y) are reserved and always read as released.
        self.input = input & 0x3F;
    }

    /// The status line set with `status(text)`; the system draws it over the
    /// bottom 9 pixel rows after `draw()`.
    pub fn status(&self) -> &[u8] {
        &self.status[..self.status_len]
    }

    pub fn draw_status(&mut self) {
        if self.status_len == 0 {
            return;
        }
        let s = self.status;
        let n = self.status_len;
        self.screen.rectfill(0, H - 9, 240, 9, 0);
        self.screen.text(&s[..n], 2, H - 9, 7);
    }

    /// Takes the next pending sound request.
    pub fn next_sound_event(&mut self) -> Option<SoundEvent> {
        let i = self.events.iter().position(|e| e.is_some())?;
        let e = self.events[i].take();
        self.events[i..].rotate_left(1);
        e
    }

    fn event(&mut self, e: SoundEvent) {
        if let Some(slot) = self.events.iter_mut().find(|x| x.is_none()) {
            *slot = Some(e);
        }
    }

    fn sheet(&self) -> &'c [u8] {
        match self.sprites {
            Some(s) if s.payload.len() >= SHEET_BYTES + TILE_COUNT => &s.payload[..SHEET_BYTES],
            _ => &[],
        }
    }

    fn sprite_slot(&self, i: i32) -> Option<(i32, i32, i32)> {
        let s = self.sprites?;
        if i < 0 || i >= s.slots as i32 {
            return None;
        }
        let b = s.slot(i as u16);
        if b.len() < 4 {
            return None;
        }
        Some((u16::from_le_bytes([b[0], b[1]]) as i32, b[2] as i32, b[3] as i32))
    }

    fn map_slot(&self, m: i32) -> Option<(i32, i32, &'c [u8])> {
        let s = self.maps?;
        if m < 0 || m >= s.slots as i32 {
            return None;
        }
        let b = s.slot(m as u16);
        if b.len() < 4 {
            return None;
        }
        let w = u16::from_le_bytes([b[0], b[1]]) as i32;
        let h = u16::from_le_bytes([b[2], b[3]]) as i32;
        if b.len() < 4 + (w * h * 2) as usize {
            return None;
        }
        Some((w, h, &b[4..]))
    }

    /// Length of a sound in milliseconds, from its 16-byte record.
    fn sound_ms(&self, slot: i32) -> Option<u32> {
        let s = self.sounds?;
        if slot < 0 || slot >= s.slots as i32 {
            return None;
        }
        let b = s.slot(slot as u16);
        if b.len() < 16 {
            return None;
        }
        let u = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]) as u32;
        let notes = b[3] as u32;
        let body = if notes == 0 { u(8) } else { notes * u(14) };
        Some(u(10) + body + u(12))
    }
}

fn cell(cells: &[u8], i: usize) -> i32 {
    u16::from_le_bytes([cells[i * 2], cells[i * 2 + 1]]) as i32
}

impl<'c> Host for Console<'c> {
    fn call(&mut self, b: u8, a: &[i32], text: &[u8], _mem: &mut Mem) -> Call {
        let c8 = |v: i32| (v & 255) as u8;
        let v = Call::Value;
        match b {
            id::CLS => {
                self.screen.cls(a.first().map_or(0, |&c| c8(c)));
                v(0)
            }
            id::PSET => {
                self.screen.px(a[0], a[1], c8(a[2]));
                v(0)
            }
            id::PGET => v(self.screen.pget(a[0], a[1]) as i32),
            id::LINE => {
                self.screen.line(a[0], a[1], a[2], a[3], c8(a[4]));
                v(0)
            }
            id::RECT => {
                self.screen.rect(a[0], a[1], a[2], a[3], c8(a[4]));
                v(0)
            }
            id::RECTFILL => {
                self.screen.rectfill(a[0], a[1], a[2], a[3], c8(a[4]));
                v(0)
            }
            id::CIRC | id::CIRCFILL => {
                self.screen.circle(a[0], a[1], a[2], c8(a[3]), b == id::CIRCFILL);
                v(0)
            }
            id::SPR => {
                let s = a[0];
                let spr = match sprite_decl(s) {
                    Some((t, w, h)) => Some((t as i32, w as i32, h as i32)),
                    None => self.sprite_slot(s),
                };
                if let Some((t, w, h)) = spr {
                    let sheet = self.sheet();
                    self.screen.sprite(sheet, t, w, h, a[1], a[2]);
                }
                v(0)
            }
            id::MAP => {
                if let Some((mw, mh, cells)) = self.map_slot(a[0]) {
                    let sheet = self.sheet();
                    let (cx, cy, x, y, w, h) = (a[1], a[2], a[3], a[4], a[5].min(1024), a[6].min(1024));
                    for j in 0..h {
                        let r = cy + j;
                        if r < 0 || r >= mh {
                            continue;
                        }
                        for i in 0..w {
                            let col = cx + i;
                            if col < 0 || col >= mw {
                                continue;
                            }
                            let t = cell(cells, (r * mw + col) as usize);
                            if t != 0 {
                                self.screen.tile(sheet, t, x.saturating_add(i * 8), y.saturating_add(j * 8));
                            }
                        }
                    }
                }
                v(0)
            }
            id::MAPCELL => {
                let (m, x, y) = (a[0], a[1], a[2]);
                v(match self.map_slot(m) {
                    Some((w, h, cells)) if x >= 0 && y >= 0 && x < w && y < h => cell(cells, (y * w + x) as usize),
                    _ => 0,
                })
            }
            id::FGET => {
                let (t, f) = (a[0], a[1]);
                v(match self.sprites {
                    Some(s) if t >= 0 && (t as usize) < TILE_COUNT && s.payload.len() >= SHEET_BYTES + TILE_COUNT => {
                        (s.payload[SHEET_BYTES + t as usize] as i32 >> (f & 7)) & 1
                    }
                    _ => 0,
                })
            }
            id::PRINT => {
                let c = a.get(3).map_or(7, |&c| c8(c));
                v(self.screen.text(text, a[1], a[2], c))
            }
            id::BTN => v(((self.input >> (a[0] & 31)) & 1) as i32),
            id::BTNP => {
                let bit = a[0] & 31;
                v((((self.input >> bit) & 1) == 1 && ((self.prev_input >> bit) & 1) == 0) as i32)
            }
            id::HASPOINTER => v(self.pointer.is_some() as i32),
            id::MOUSEX => v(self.pointer.map_or(0, |p| p.0)),
            id::MOUSEY => v(self.pointer.map_or(0, |p| p.1)),
            id::MOUSEB => v(self.pointer.map_or(0, |p| p.2)),
            id::SETPAL => {
                if !(0..=255).contains(&a[0]) {
                    return Call::Fault("palette index must be 0 to 255");
                }
                self.screen.setpal(a[0] as u8, a[1], a[2], a[3]);
                v(0)
            }
            id::GETPAL => {
                if !(0..=2).contains(&a[1]) {
                    return Call::Fault("channel must be 0 (red), 1 (green) or 2 (blue)");
                }
                v(self.screen.getpal(c8(a[0]), a[1] as usize))
            }
            id::PALT => {
                self.screen.transparent = c8(a[0]);
                v(0)
            }
            id::PLAY => {
                let Some(ms) = self.sound_ms(a[0]) else { return v(-1) };
                let voice = if a.len() > 1 {
                    if !(0..4).contains(&a[1]) {
                        return Call::Fault("voice must be 0 to 3");
                    }
                    a[1] as usize
                } else {
                    let now = self.now_ms;
                    match self.voices.iter().position(|x| x.1 <= now) {
                        Some(i) => i,
                        None => (0..4).min_by_key(|&i| self.voices[i].0).unwrap(),
                    }
                };
                self.voices[voice] = (self.now_ms, self.now_ms + ms);
                self.event(SoundEvent::Play {
                    voice: voice as u8,
                    slot: a[0] as u16,
                });
                v(voice as i32)
            }
            id::STOP => {
                if !(0..4).contains(&a[0]) {
                    return Call::Fault("voice must be 0 to 3");
                }
                self.voices[a[0] as usize].1 = self.now_ms;
                self.event(SoundEvent::Stop { voice: a[0] as u8 });
                v(0)
            }
            id::STOPALL => {
                for i in 0..4 {
                    self.voices[i].1 = self.now_ms;
                    self.event(SoundEvent::Stop { voice: i as u8 });
                }
                v(0)
            }
            id::PLAYING => {
                if !(0..4).contains(&a[0]) {
                    return Call::Fault("voice must be 0 to 3");
                }
                v((self.voices[a[0] as usize].1 > self.now_ms) as i32)
            }
            id::CUE | id::CUECOUNT => {
                if !(0..8).contains(&a[0]) {
                    return Call::Fault("cue must be 0 to 7");
                }
                if b == id::CUE {
                    self.cues[a[0] as usize] = self.cues[a[0] as usize].wrapping_add(1);
                    v(0)
                } else {
                    v(self.cues[a[0] as usize])
                }
            }
            id::STATUS => {
                let n = text.len().min(STATUS_MAX);
                self.status[..n].copy_from_slice(&text[..n]);
                self.status_len = n;
                v(0)
            }
            id::PICKCART => {
                self.dialog = Some(Dialog::PickCart);
                Call::Suspend
            }
            id::SAVECART => {
                self.dialog = Some(Dialog::SaveCart { handle: a[0] });
                Call::Suspend
            }
            // The rest of the editing API needs the platform's cart shelf.
            id::OPENSECTION | id::OPENCART => v(-1),
            id::CARTCOUNT | id::CARTINFO => v(0),
            id::REQUESTWRITE => {
                self.dialog = Some(Dialog::RequestWrite { handle: a[0] });
                Call::Suspend
            }
            id::CARTNAME | id::ASSETCOUNT | id::ASSETNAME | id::READASSET | id::WRITEASSET | id::SETASSETNAME | id::DRAWASSET => v(0),
            _ => Call::Fault("unknown built-in"),
        }
    }
}
