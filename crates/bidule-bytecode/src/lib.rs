//! Bidule 01 bytecode definition, shared by the compiler and the VM.
//!
//! This crate is the single source of truth for the byte-level format described
//! in `spec/bytecode.md`: opcodes, built-in ids, the `.bdb` container and its
//! sections, array handles, and the load-time verifier.
#![no_std]

#[cfg(feature = "alloc")]
extern crate alloc;

pub mod asset;
pub mod builtins;
pub mod handle;
pub mod image;
pub mod op;
pub mod verify;

pub use image::{Cart, LoadError, Program};
pub use op::Op;

/// Fixed limits of Bidule 01 (spec: Language › Limits, Runtime, VM and bytecode).
pub mod limits {
    pub const GLOBALS: usize = 256;
    pub const ARRAYS: usize = 255;
    /// Array pool, in 32-bit elements (16 KB).
    pub const POOL: usize = 4096;
    pub const AUDIO_VARS: usize = 64;
    /// Locals and parameters of one function.
    pub const LOCALS: usize = 64;
    /// Local slots across all active frames (2 KB).
    pub const LOCAL_STACK: usize = 512;
    pub const CALL_DEPTH: usize = 16;
    /// Value stack shared by all active frames (2 KB).
    pub const VALUE_STACK: usize = 512;
    /// Deepest stack one function may declare.
    pub const MAX_STACK: usize = 254;
    /// Audio core: locals and value stack.
    pub const AUDIO_LOCALS: usize = 64;
    pub const AUDIO_STACK: usize = 64;
    pub const FUNCTIONS: usize = 1024;
    pub const ROM_TABLES: usize = 1024;
    pub const TABLE_ELEMS: usize = 65535;
    pub const TEMPLATES: usize = 1024;
    pub const TEMPLATE_ARGS: usize = 16;
    /// Template arguments waiting for their built-in, across nested calls.
    pub const FMT_STACK: usize = 64;
    /// Longest formatted text, in bytes; longer text is cut.
    pub const TEXT_BYTES: usize = 512;
    /// Bytecode of one function.
    pub const FN_CODE_BYTES: usize = 16 * 1024;
    /// Whole code section.
    pub const CODE_BYTES: usize = 256 * 1024;
    /// Whole cart image. Proposed until the flash size is chosen.
    pub const CART_BYTES: usize = 4 * 1024 * 1024;
}

/// Compiler version stamped in carts: major << 8 | minor.
pub const COMPILER_VERSION: u16 = 0x0001;
