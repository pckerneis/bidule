//! Bidule 01 console runtime, shared by every platform: the renderer, the
//! Monogram font and the system side of the cart API ([`console::Console`],
//! the VM's host). `no_std`, no allocation.
#![no_std]

pub mod console;
pub mod font;
pub mod gfx;

pub use console::{Console, Dialog, SoundEvent};
pub use gfx::{Screen, H, W};
