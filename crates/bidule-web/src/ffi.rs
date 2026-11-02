//! The WebAssembly interface. Numbers go by value; bytes go through linear
//! memory, at pointers exchanged here. `src/js/wasm.js` is the other side.
//!
//! The page makes one instance per cart and never calls back into it during
//! `shelf_call`, so the session is simply a thread-local.

use std::cell::RefCell;
use std::slice;

use bidule_bytecode::image::Handler;
use bidule_compiler::Assets;

use crate::{build, set_text, Answer, Session, Shelf};

#[link(wasm_import_module = "bidule")]
extern "C" {
    /// Runs an editing API built-in in the page. A non-zero `*fault` stops the cart.
    #[allow(clippy::too_many_arguments)]
    fn shelf_call(
        id: i32,
        args: *const i32,
        nargs: i32,
        text: *const u8,
        text_len: i32,
        buf: *mut u8,
        buf_cap: i32,
        buf_len: i32,
        fault: *mut i32,
    ) -> i32;
}

struct PageShelf;

impl Shelf for PageShelf {
    fn call(&mut self, id: u8, args: &[i32], text: &[u8], buf: &mut [u8], len: usize) -> Answer {
        let mut fault = 0;
        // SAFETY: the page reads and writes only inside these live buffers, within
        // the lengths given, before returning.
        let v = unsafe {
            shelf_call(
                id as i32,
                args.as_ptr(),
                args.len() as i32,
                text.as_ptr(),
                text.len() as i32,
                buf.as_mut_ptr(),
                buf.len() as i32,
                len as i32,
                &mut fault,
            )
        };
        if fault != 0 {
            Answer::Fault
        } else {
            Answer::Value(v)
        }
    }
}

#[derive(Default)]
struct State {
    image: Option<Vec<u8>>,
    session: Option<Box<Session<PageShelf>>>,
    text: String,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

fn session<R>(f: impl FnOnce(&mut Session<PageShelf>) -> R) -> R {
    with(|s| f(s.session.as_mut().expect("no cart loaded")))
}

/// Memory for the page to write source and assets into; kept until the instance goes.
#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    Vec::leak(vec![0u8; len]).as_mut_ptr()
}

/// # Safety
/// `(ptr, len)` must come from [`alloc`], or `len` must be 0.
unsafe fn bytes<'a>(ptr: *const u8, len: usize) -> Option<&'a [u8]> {
    (len != 0).then(|| slice::from_raw_parts(ptr, len))
}

/// Compiles a cart; returns 1 on success. The text holds diagnostics either way.
///
/// # Safety
/// Every pointer and length pair must come from [`alloc`], or have length 0.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn compile(
    source: *const u8,
    source_len: usize,
    sprites: *const u8,
    sprites_len: usize,
    maps: *const u8,
    maps_len: usize,
    sounds: *const u8,
    sounds_len: usize,
    palette: *const u8,
    palette_len: usize,
) -> i32 {
    let source = String::from_utf8_lossy(bytes(source, source_len).unwrap_or(&[]));
    let assets = Assets {
        sprites: bytes(sprites, sprites_len),
        maps: bytes(maps, maps_len),
        sounds: bytes(sounds, sounds_len),
        music: None,
        palette: bytes(palette, palette_len),
    };
    let built = build(&source, &assets);
    with(|s| match built {
        Ok((image, warnings)) => {
            s.image = Some(image);
            s.text = warnings;
            1
        }
        Err(errors) => {
            s.text = errors;
            0
        }
    })
}

/// Verifies the compiled image; returns 1 on success, or 0 with the reason in the text.
#[no_mangle]
pub extern "C" fn load(debug: i32, seed: u32) -> i32 {
    with(|s| {
        let image = s.image.take().unwrap_or_default();
        match Session::load(image, debug != 0, seed, PageShelf) {
            Ok(session) => {
                s.session = Some(session);
                1
            }
            Err(e) => {
                set_text(&mut s.text, &e);
                0
            }
        }
    })
}

#[no_mangle]
pub extern "C" fn text_ptr() -> *const u8 {
    with(|s| s.text.as_ptr())
}

#[no_mangle]
pub extern "C" fn text_len() -> usize {
    with(|s| s.text.len())
}

#[no_mangle]
pub extern "C" fn begin_frame(input: u32, frame: u32, x: i32, y: i32, b: i32) {
    session(|s| {
        s.begin_frame(input, frame);
        s.host.console.pointer = Some((x, y, b));
    })
}

/// The pointer changes while a system dialog holds a frame open.
#[no_mangle]
pub extern "C" fn set_pointer(x: i32, y: i32, b: i32) {
    session(|s| s.host.console.pointer = Some((x, y, b)))
}

/// Starts a handler (0 init, 1 update, 2 draw); returns 1 if the cart defines it.
#[no_mangle]
pub extern "C" fn start(handler: i32, frame: u32, input: u32) -> i32 {
    let h = match handler {
        0 => Handler::Init,
        1 => Handler::Update,
        _ => Handler::Draw,
    };
    session(|s| s.start(h, frame, input) as i32)
}

/// 0 done, 1 late, 2 dialog, 3 fault.
#[no_mangle]
pub extern "C" fn run() -> i32 {
    let step = session(|s| s.run());
    with(|s| {
        if let Some(session) = &s.session {
            s.text.clear();
            s.text.push_str(session.message());
        }
    });
    step as i32
}

#[no_mangle]
pub extern "C" fn resume(value: i32) {
    session(|s| s.resume(value))
}

#[no_mangle]
pub extern "C" fn fault_line() -> i32 {
    session(|s| s.fault_line())
}

#[no_mangle]
pub extern "C" fn fb_ptr() -> *const u8 {
    session(|s| s.host.console.screen.fb.as_ptr())
}

#[no_mangle]
pub extern "C" fn pal_ptr() -> *const u16 {
    session(|s| s.host.console.screen.pal.as_ptr())
}

#[no_mangle]
pub extern "C" fn status_ptr() -> *const u8 {
    session(|s| s.host.console.status().as_ptr())
}

#[no_mangle]
pub extern "C" fn status_len() -> usize {
    session(|s| s.host.console.status().len())
}

#[no_mangle]
pub extern "C" fn next_sound() -> i32 {
    session(|s| s.next_sound())
}

#[no_mangle]
pub extern "C" fn has_audio() -> i32 {
    session(|s| s.has_audio() as i32)
}

/// One bytebeat sample; bit 8 is set the first time `audio(t)` faults, with the
/// message in the text and the line in [`fault_line`].
#[no_mangle]
pub extern "C" fn audio_sample(t: i32) -> i32 {
    let (v, fault) = session(|s| s.audio_sample(t));
    if fault {
        with(|s| {
            if let Some(session) = &s.session {
                s.text.clear();
                s.text.push_str(session.message());
            }
        });
    }
    v as i32 | (fault as i32) << 8
}
