//! Asset sections (spec: Storage › Asset section layout › Common container).
//!
//! | Part | Content |
//! | --- | --- |
//! | Header (16 B) | Kind (u8), format version (u8), revision (u16), slot count (u16), flags (u16), payload CRC-32 (u32), reserved (4 B) |
//! | Name table | Per slot: name length (u8), then the name |
//! | Slot table | Per slot: offset (u32) and length (u32) into the payload |
//! | Payload | Kind-specific data |
//!
//! The kind byte is the low byte of the `.bdb` section kind (0x10 sprites,
//! 0x11 maps, 0x12 sounds, 0x13 music, 0x14 palette).

use crate::image::{crc32, u16_at, u32_at};

pub const HEADER: usize = 16;
pub const FORMAT_VERSION: u8 = 1;
pub const MAX_NAME: usize = 24;

/// Sprite sheet geometry.
pub const TILE_COUNT: usize = 1024;
pub const SHEET_COLS: usize = 32;
pub const SHEET_BYTES: usize = TILE_COUNT * 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetError {
    TooShort,
    BadHeader,
    BadNames,
    BadSlots,
    BadChecksum,
}

impl core::fmt::Display for AssetError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            AssetError::TooShort => "asset section too short",
            AssetError::BadHeader => "bad asset section header",
            AssetError::BadNames => "bad asset name table",
            AssetError::BadSlots => "bad asset slot table",
            AssetError::BadChecksum => "asset payload checksum mismatch",
        })
    }
}

/// A checked asset section, read in place.
#[derive(Clone, Copy, Debug)]
pub struct AssetSection<'a> {
    pub kind: u8,
    pub revision: u16,
    pub slots: u16,
    names: &'a [u8],
    slot_table: &'a [u8],
    pub payload: &'a [u8],
}

/// Valid asset or identifier name: letters, digits and `_`, not starting with a digit.
pub fn valid_name(n: &[u8]) -> bool {
    !n.is_empty() && n.len() <= MAX_NAME && !n[0].is_ascii_digit() && n.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_')
}

impl<'a> AssetSection<'a> {
    pub fn parse(b: &'a [u8]) -> Result<AssetSection<'a>, AssetError> {
        if b.len() < HEADER {
            return Err(AssetError::TooShort);
        }
        let kind = b[0];
        if !(0x10..=0x14).contains(&kind) || b[1] != FORMAT_VERSION || u16_at(b, 6) != 0 || u32_at(b, 12) != 0 {
            return Err(AssetError::BadHeader);
        }
        let revision = u16_at(b, 2);
        let slots = u16_at(b, 4);
        let mut p = HEADER;
        for _ in 0..slots {
            let len = *b.get(p).ok_or(AssetError::BadNames)? as usize;
            let name = b.get(p + 1..p + 1 + len).ok_or(AssetError::BadNames)?;
            if len != 0 && !valid_name(name) {
                return Err(AssetError::BadNames);
            }
            p += 1 + len;
        }
        let names = &b[HEADER..p];
        let st_end = p + slots as usize * 8;
        let slot_table = b.get(p..st_end).ok_or(AssetError::BadSlots)?;
        let payload = &b[st_end..];
        if crc32(payload) != u32_at(b, 8) {
            return Err(AssetError::BadChecksum);
        }
        for i in 0..slots as usize {
            let off = u32_at(slot_table, i * 8) as usize;
            let len = u32_at(slot_table, i * 8 + 4) as usize;
            if off.checked_add(len).is_none_or(|e| e > payload.len()) {
                return Err(AssetError::BadSlots);
            }
        }
        Ok(AssetSection {
            kind,
            revision,
            slots,
            names,
            slot_table,
            payload,
        })
    }

    /// Name of slot `i` (empty when unnamed).
    pub fn name(&self, i: u16) -> &'a [u8] {
        let mut p = 0;
        for _ in 0..i {
            p += 1 + self.names[p] as usize;
        }
        let len = self.names[p] as usize;
        &self.names[p + 1..p + 1 + len]
    }

    /// Bytes of slot `i`; empty for an emptied slot.
    pub fn slot(&self, i: u16) -> &'a [u8] {
        if i >= self.slots {
            return &[];
        }
        let off = u32_at(self.slot_table, i as usize * 8) as usize;
        let len = u32_at(self.slot_table, i as usize * 8 + 4) as usize;
        &self.payload[off..off + len]
    }
}

#[cfg(feature = "alloc")]
pub mod write {
    use super::*;
    use crate::image::write::{put_u16, put_u32};
    use alloc::vec::Vec;

    /// Builds an asset section. `fixed` is payload that precedes the slots
    /// (the sprite sheet and flags); each slot is `(name, bytes)`.
    pub fn section(kind: u8, revision: u16, fixed: &[u8], slots: &[(&[u8], &[u8])]) -> Vec<u8> {
        let mut payload = Vec::from(fixed);
        let mut table = Vec::new();
        for (_, data) in slots {
            put_u32(&mut table, if data.is_empty() { 0 } else { payload.len() as u32 });
            put_u32(&mut table, data.len() as u32);
            payload.extend_from_slice(data);
        }
        let mut out = Vec::new();
        out.push(kind);
        out.push(FORMAT_VERSION);
        put_u16(&mut out, revision);
        put_u16(&mut out, slots.len() as u16);
        put_u16(&mut out, 0);
        put_u32(&mut out, crc32(&payload));
        put_u32(&mut out, 0);
        for (name, _) in slots {
            out.push(name.len() as u8);
            out.extend_from_slice(name);
        }
        out.extend_from_slice(&table);
        out.extend_from_slice(&payload);
        out
    }
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;
    #[test]
    fn round_trip() {
        let b = write::section(0x12, 3, &[], &[(b"JUMP", &[0u8; 16]), (b"", &[]), (b"COIN", &[1u8; 18])]);
        let s = AssetSection::parse(&b).unwrap();
        assert_eq!((s.kind, s.revision, s.slots), (0x12, 3, 3));
        assert_eq!(s.name(0), b"JUMP");
        assert_eq!(s.name(1), b"");
        assert_eq!(s.name(2), b"COIN");
        assert_eq!(s.slot(1).len(), 0);
        assert_eq!(s.slot(2), &[1u8; 18]);
        let mut t = b.clone();
        *t.last_mut().unwrap() ^= 1;
        assert_eq!(AssetSection::parse(&t).unwrap_err(), AssetError::BadChecksum);
    }
}
