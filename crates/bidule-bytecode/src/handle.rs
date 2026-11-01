//! Array references ("handles").
//!
//! A handle is an ordinary 32-bit value, so a cart can forge any bit pattern;
//! the VM decodes and bounds-checks every handle on use.
//!
//! | Bits  | Field |
//! | ----- | ----- |
//! | 0–15  | Offset: first element (for a row, `row * cols`; for a template, the base of its arguments) |
//! | 16–25 | Id: pool array (0–254), constant table (0–1023) or template (0–1023) |
//! | 26–27 | Kind: 0 pool array, 1 constant table, 2 text template, 3 null |
//! | 28    | Row: the handle covers exactly one row (`cols` elements) of a 2D array |
//! | 29–31 | Must be 0 |

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Pool,
    Rom,
    Template,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handle {
    pub kind: Kind,
    pub id: u16,
    pub offset: u16,
    pub row: bool,
}

pub const NULL: i32 = 3 << 26;
pub const ID_SHIFT: u32 = 16;

impl Handle {
    pub const fn pool(id: u16) -> Handle {
        Handle {
            kind: Kind::Pool,
            id,
            offset: 0,
            row: false,
        }
    }
    pub const fn rom(id: u16) -> Handle {
        Handle {
            kind: Kind::Rom,
            id,
            offset: 0,
            row: false,
        }
    }
    pub const fn encode(self) -> i32 {
        let k = match self.kind {
            Kind::Pool => 0,
            Kind::Rom => 1,
            Kind::Template => 2,
        };
        ((self.row as u32) << 28 | k << 26 | (self.id as u32 & 0x3FF) << 16 | self.offset as u32) as i32
    }
    /// Decodes a value; `None` for the null handle and any malformed value.
    pub const fn decode(v: i32) -> Option<Handle> {
        let v = v as u32;
        if v >> 29 != 0 {
            return None;
        }
        let kind = match (v >> 26) & 3 {
            0 => Kind::Pool,
            1 => Kind::Rom,
            2 => Kind::Template,
            _ => return None,
        };
        let row = (v >> 28) & 1 == 1;
        if row && matches!(kind, Kind::Template) {
            return None;
        }
        Some(Handle {
            kind,
            id: ((v >> 16) & 0x3FF) as u16,
            offset: (v & 0xFFFF) as u16,
            row,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip() {
        for h in [
            Handle::pool(0),
            Handle::pool(254),
            Handle::rom(1023),
            Handle {
                kind: Kind::Pool,
                id: 3,
                offset: 48,
                row: true,
            },
            Handle {
                kind: Kind::Template,
                id: 7,
                offset: 12,
                row: false,
            },
        ] {
            assert_eq!(Handle::decode(h.encode()), Some(h));
        }
        assert_eq!(Handle::decode(NULL), None);
        assert_eq!(Handle::decode(-1), None);
        // a field reference is the record's first field plus k in the id
        let base = Handle::pool(10).encode();
        assert_eq!(Handle::decode(base + (2 << ID_SHIFT)), Some(Handle::pool(12)));
    }
}
