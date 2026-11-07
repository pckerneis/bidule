//! The Bidule 01 cart compiler.
//!
//! One implementation for every target (spec: Storage › Compiler constraints):
//! `no_std` with `alloc`, deterministic, and meant to fit in 128 KB of working RAM.
//!
//! ```text
//! .bdcart source ─ lexer ─ parser ─ declarations ─ code generation ─ .bdb image
//! ```
//!
//! To stay small, the syntax tree is never in memory as a whole. The parser
//! checks the source and keeps, for each top-level item, its kind, name and
//! position ([`ast::ItemAt`]); later passes parse an item again with
//! [`parser::item_at`] when they need it, and read function bodies one
//! statement at a time with [`parser::BodyReader`]. Names are slices of the
//! source, and the syntax tree and symbol tables are freed before the image is
//! written.
#![no_std]

extern crate alloc;

pub mod ast;
pub mod compile;
pub mod diag;
pub mod fold;
pub mod gen;
pub mod lexer;
pub mod parser;
pub mod types;

use alloc::vec::Vec;

pub use compile::{Assets, Options, Output};
pub use diag::{Diag, Pos, Severity};

/// Compiles a cart's source and asset sections into a `.bdb` image.
///
/// On success, returns the image and any warnings; on failure, every
/// diagnostic found (errors and warnings), in source order.
pub fn compile(source: &str, assets: &Assets, opts: &Options) -> Result<Output, Vec<Diag>> {
    let (prog, mut diags) = match parser::parse(source) {
        Ok(x) => x,
        Err(e) => return Err(alloc::vec![e]),
    };
    let mut c = compile::Compiler::new(&prog, source, assets, *opts);
    let ok = c.run();
    diags.append(&mut c.diags);
    let cart = if ok {
        // free the syntax tree and the symbol tables before writing the image
        let parts = c.into_parts();
        drop(prog);
        match parts.assemble() {
            Ok(b) => Some(b),
            Err(d) => {
                diags.push(d);
                None
            }
        }
    } else {
        None
    };
    diags.sort_by_key(|d| (d.pos.line, d.pos.col));
    match cart {
        Some(cart) => Ok(Output { cart, warnings: diags }),
        None => Err(diags),
    }
}
