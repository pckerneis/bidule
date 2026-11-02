//! Bidule 01 for the browser: the compiler, VM and runtime behind a plain
//! WebAssembly interface (no generated bindings, see `src/js/wasm.js`).
//!
//! The page owns the cart shelf, the system dialogs and the audio output. Host
//! built-ins go to [`Console`]; the editing API goes to the page through
//! [`Shelf`], because it reads and writes carts other than the running one.
//!
//! The logic lives here and is tested natively; [`ffi`] only moves bytes.

#[cfg(target_arch = "wasm32")]
mod ffi;

use bidule_bytecode::builtins::id;
use bidule_bytecode::image::{Cart, Handler};
use bidule_bytecode::limits;
use bidule_compiler::{compile, Assets, Options};
use bidule_runtime::{Console, SoundEvent};
use bidule_vm::{Call, Exit, Fault, Host, Mem, Vm};

/// Instructions per [`Session::run`]; what is left continues on the next frame
/// (bytecode.md › Execution).
pub const FRAME_BUDGET: u32 = 2_000_000;

/// Room for the largest exchange with the page: a slot of a cart's array pool,
/// or the tiles of a sprite as large as the sheet is wide.
const EXCHANGE_BYTES: usize = if limits::POOL > 32 * 32 * 64 { limits::POOL } else { 32 * 32 * 64 };

/// What the page answers to an editing API call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Value(i32),
    /// The call stops the cart; the page holds the message.
    Fault,
}

/// The page's side of the editing API (spec: Storage › Editing API).
pub trait Shelf {
    /// Runs built-in `id`. `len` is how many bytes of `buf` the VM filled
    /// (`writeasset`); the page may write its result to `buf`, up to its length.
    fn call(&mut self, id: u8, args: &[i32], text: &[u8], buf: &mut [u8], len: usize) -> Answer;
}

/// The VM's host: the console, with the editing API sent to the page.
pub struct WebHost<S> {
    pub console: Console<'static>,
    pub shelf: S,
    buf: Vec<u8>,
}

const SHELF_FAULT: &str = "editing API error";

impl<S: Shelf> WebHost<S> {
    fn ask(&mut self, b: u8, a: &[i32], text: &[u8], len: usize) -> Answer {
        self.shelf.call(b, a, text, &mut self.buf, len)
    }

    /// `cartname`, `assetname`: text for an array, NUL-terminated as `fmt` does.
    fn name_into(&mut self, b: u8, a: &[i32], dst: i32, mem: &mut Mem) -> Call {
        let Some(extent) = mem.extent(dst) else {
            return Call::Fault("not an array");
        };
        let Answer::Value(n) = self.ask(b, a, &[], 0) else {
            return Call::Fault(SHELF_FAULT);
        };
        let n = (n.max(0) as usize).min(self.buf.len()).min(extent.saturating_sub(1));
        for i in 0..n {
            if !mem.set(dst, i, self.buf[i] as i32) {
                return Call::Fault("write to read-only data");
            }
        }
        if extent > 0 && !mem.set(dst, n, 0) {
            return Call::Fault("write to read-only data");
        }
        Call::Value(n as i32)
    }

    fn read_asset(&mut self, a: &[i32], mem: &mut Mem) -> Call {
        let Some(extent) = mem.extent(a[1]) else {
            return Call::Fault("not an array");
        };
        let Answer::Value(n) = self.ask(id::READASSET, a, &[], 0) else {
            return Call::Fault(SHELF_FAULT);
        };
        for i in 0..(n.max(0) as usize).min(extent).min(self.buf.len()) {
            if !mem.set(a[1], i, self.buf[i] as i32) {
                return Call::Fault("write to read-only data");
            }
        }
        Call::Value(n)
    }

    fn write_asset(&mut self, a: &[i32], mem: &mut Mem) -> Call {
        let Some(extent) = mem.extent(a[1]) else {
            return Call::Fault("not an array");
        };
        let n = extent.min(self.buf.len());
        for i in 0..n {
            self.buf[i] = mem.get(a[1], i).unwrap_or(0) as u8;
        }
        match self.ask(id::WRITEASSET, a, &[], n) {
            Answer::Value(v) => Call::Value(v),
            Answer::Fault => Call::Fault(SHELF_FAULT),
        }
    }

    /// The page answers `w | h << 8` and puts the tiles, 64 bytes each, in the buffer.
    fn draw_asset(&mut self, a: &[i32]) -> Call {
        let Answer::Value(size) = self.ask(id::DRAWASSET, a, &[], 0) else {
            return Call::Fault(SHELF_FAULT);
        };
        let (w, h) = (size & 255, (size >> 8) & 255);
        let (x, y) = (a[2], a[3]);
        for ty in 0..h {
            for tx in 0..w {
                self.console
                    .screen
                    .tile(&self.buf, ty * w + tx, x.saturating_add(tx * 8), y.saturating_add(ty * 8));
            }
        }
        Call::Value(0)
    }
}

impl<S: Shelf> Host for WebHost<S> {
    fn call(&mut self, b: u8, a: &[i32], text: &[u8], mem: &mut Mem) -> Call {
        match b {
            id::PICKCART | id::SAVECART => match self.ask(b, a, text, 0) {
                Answer::Value(_) => Call::Suspend,
                Answer::Fault => Call::Fault(SHELF_FAULT),
            },
            id::OPENSECTION | id::ASSETCOUNT | id::SETASSETNAME => match self.ask(b, a, text, 0) {
                Answer::Value(v) => Call::Value(v),
                Answer::Fault => Call::Fault(SHELF_FAULT),
            },
            id::CARTNAME => self.name_into(b, a, a[1], mem),
            id::ASSETNAME => self.name_into(b, a, a[2], mem),
            id::READASSET => self.read_asset(a, mem),
            id::WRITEASSET => self.write_asset(a, mem),
            id::DRAWASSET => self.draw_asset(a),
            _ => self.console.call(b, a, text, mem),
        }
    }
}

/// How [`Session::run`] stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Step {
    Done = 0,
    /// Out of budget for this frame; run again on the next one.
    Late = 1,
    /// Waiting on a system dialog; call [`Session::resume`].
    Dialog = 2,
    /// Stopped; see [`Session::fault_line`] and [`Session::message`].
    Fault = 3,
}

/// Compiles source and assets to a cart image, with its warnings. On failure,
/// the error text has one diagnostic per line, as `line:col: error: message`.
pub fn build(source: &str, assets: &Assets) -> Result<(Vec<u8>, String), String> {
    let render = |ds: &[bidule_compiler::Diag]| ds.iter().map(|d| format!("{}\n", d)).collect::<String>();
    match compile(source, assets, &Options { debug_info: true }) {
        Ok(o) => Ok((o.cart, render(&o.warnings))),
        Err(ds) => Err(render(&ds)),
    }
}

/// A loaded cart. The page makes a new WebAssembly instance for each cart, so
/// the image lives as long as the instance.
pub struct Session<S> {
    vm: Vm<'static>,
    pub host: WebHost<S>,
    message: String,
    fault_line: i32,
    audio_reported: bool,
}

impl<S: Shelf> Session<S> {
    /// Verifies a cart image and makes it ready to run.
    pub fn load(image: Vec<u8>, debug: bool, seed: u32, shelf: S) -> Result<Box<Self>, String> {
        let image: &'static [u8] = Box::leak(image.into_boxed_slice());
        let mut scratch = vec![0u8; limits::FN_CODE_BYTES];
        let vm = Vm::load(image, &mut scratch, debug, seed).map_err(|e| e.to_string())?;
        let cart = Cart::parse(image).map_err(|e| e.to_string())?;
        let mut console = Console::new(&cart);
        console.pointer = Some((0, 0, 0));
        Ok(Box::new(Session {
            vm,
            host: WebHost {
                console,
                shelf,
                buf: vec![0; EXCHANGE_BYTES],
            },
            message: String::new(),
            fault_line: 0,
            audio_reported: false,
        }))
    }

    /// Starts a frame: this frame's input, and the time carts see.
    pub fn begin_frame(&mut self, input: u32, frame: u32) {
        self.host.console.begin_frame(input);
        self.host.console.now_ms = (frame as u64 * 1000 / 60) as u32;
        self.vm.snapshot_globals();
    }

    /// Starts a handler; false if the cart doesn't define it.
    pub fn start(&mut self, h: Handler, frame: u32, input: u32) -> bool {
        self.vm.start(h, &[frame as i32, input as i32])
    }

    pub fn run(&mut self) -> Step {
        match self.vm.run(&mut self.host, FRAME_BUDGET) {
            Exit::Done(_) => Step::Done,
            Exit::Budget => Step::Late,
            Exit::Suspend { .. } => Step::Dialog,
            Exit::Fault(f) => {
                self.record_fault(&f);
                Step::Fault
            }
        }
    }

    pub fn resume(&mut self, value: i32) {
        self.vm.resume(value);
    }

    fn record_fault(&mut self, f: &Fault) {
        self.message.clear();
        let _ = self.vm.write_fault(&mut self.message, f);
        self.fault_line = self.vm.fault_line(f).map_or(0, |(l, _)| l as i32);
    }

    /// One bytebeat sample. The second value is true the first time `audio(t)`
    /// faults, with [`Session::message`] and [`Session::fault_line`] set.
    pub fn audio_sample(&mut self, t: i32) -> (u8, bool) {
        let v = self.vm.audio_sample(t, &mut self.host);
        if self.audio_reported {
            return (v, false);
        }
        match self.vm.audio_fault {
            Some(f) => {
                self.audio_reported = true;
                self.record_fault(&f);
                (v, true)
            }
            None => (v, false),
        }
    }

    pub fn has_audio(&self) -> bool {
        self.vm.has(Handler::Audio)
    }
    pub fn message(&self) -> &str {
        &self.message
    }
    pub fn fault_line(&self) -> i32 {
        self.fault_line
    }

    /// The next sound request as `kind << 24 | voice << 16 | slot` (kind 1 is
    /// play, 2 is stop), or -1.
    pub fn next_sound(&mut self) -> i32 {
        match self.host.console.next_sound_event() {
            Some(SoundEvent::Play { voice, slot }) => 1 << 24 | (voice as i32) << 16 | slot as i32,
            Some(SoundEvent::Stop { voice }) => 2 << 24 | (voice as i32) << 16,
            None => -1,
        }
    }
}

/// Replaces the text the page reads.
pub fn set_text(out: &mut String, text: &str) {
    out.clear();
    out.push_str(text);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shelf with one cart: tile bytes read from and written to memory.
    #[derive(Default)]
    struct TestShelf {
        calls: Vec<(u8, Vec<i32>, Vec<u8>)>,
        written: Vec<u8>,
        fail: bool,
    }

    impl Shelf for TestShelf {
        fn call(&mut self, b: u8, a: &[i32], text: &[u8], buf: &mut [u8], len: usize) -> Answer {
            self.calls.push((b, a.to_vec(), text.to_vec()));
            if self.fail {
                return Answer::Fault;
            }
            Answer::Value(match b {
                id::PICKCART => 0,
                id::READASSET => {
                    buf[..3].copy_from_slice(&[7, 8, 9]);
                    3
                }
                id::WRITEASSET => {
                    self.written = buf[..len].to_vec();
                    1
                }
                id::CARTNAME => {
                    buf[..5].copy_from_slice(b"Spark");
                    5
                }
                id::DRAWASSET => {
                    buf[..64].fill(5);
                    buf[64..128].fill(6);
                    2 | 1 << 8
                }
                id::ASSETCOUNT => 42,
                _ => 0,
            })
        }
    }

    fn session(src: &str) -> Box<Session<TestShelf>> {
        let (image, _) = build(src, &Assets::default()).unwrap();
        Session::load(image, true, 1, TestShelf::default()).unwrap()
    }

    fn finish(s: &mut Session<TestShelf>, h: Handler) -> Step {
        if !s.start(h, 0, 0) {
            return Step::Done;
        }
        s.run()
    }

    #[test]
    fn compile_errors_have_positions() {
        let e = build("draw() {\n  nope()\n}", &Assets::default()).unwrap_err();
        assert!(e.starts_with("2:"), "{}", e);
        assert!(e.contains("error"), "{}", e);
    }

    #[test]
    fn dialog_suspends_and_resumes_with_the_answer() {
        let mut s = session("var h = 5\ninit() {\n  h = pickcart(\"tiles\")\n  cls(h + 1)\n}");
        assert_eq!(finish(&mut s, Handler::Init), Step::Dialog);
        let (b, _, text) = &s.host.shelf.calls[0];
        assert_eq!((*b, text.as_slice()), (id::PICKCART, &b"tiles"[..]));
        s.resume(3);
        assert_eq!(s.run(), Step::Done);
        assert_eq!(s.host.console.screen.fb[0], 4);
    }

    #[test]
    fn assets_move_through_arrays() {
        let mut s = session(
            "var a[5]\nvar n[3]\nvar got\ninit() {\n  got = readasset(1, a, 0)\n  a[1] = 300\n  writeasset(1, a, 0)\n  got = got * 100 + cartname(1, n)\n  cls(a[2])\n  pset(1, 0, got / 10)\n  pset(2, 0, n[0])\n  pset(3, 0, n[2])\n}",
        );
        assert_eq!(finish(&mut s, Handler::Init), Step::Done);
        assert_eq!(s.host.shelf.written, [7, 44, 9, 0, 0]);
        let fb = &s.host.console.screen.fb;
        assert_eq!(fb[0], 9);
        assert_eq!(fb[1], 30); // 3 bytes read, then 2 of "Spark" fit with the NUL
        assert_eq!(fb[2], b'S');
        assert_eq!(fb[3], 0);
    }

    #[test]
    fn drawasset_draws_what_the_page_sends() {
        let mut s = session("init() {\n  drawasset(1, 0, 8, 0)\n}");
        assert_eq!(finish(&mut s, Handler::Init), Step::Done);
        let fb = &s.host.console.screen.fb;
        assert_eq!((fb[7], fb[8], fb[16]), (0, 5, 6));
    }

    #[test]
    fn page_faults_stop_the_cart_on_its_line() {
        let mut s = session("init() {\n  cls(0)\n  assetcount(9)\n}");
        s.host.shelf.fail = true;
        assert_eq!(finish(&mut s, Handler::Init), Step::Fault);
        assert_eq!(s.fault_line(), 3);
        assert_eq!(s.message(), SHELF_FAULT);
    }

    #[test]
    fn a_runaway_handler_reports_late_and_keeps_going() {
        let mut s = session("var i\nupdate() {\n  while (i < 3000000) {\n    i = i + 1\n  }\n  cls(9)\n}");
        assert_eq!(finish(&mut s, Handler::Update), Step::Late);
        let mut steps = 0;
        while s.run() == Step::Late {
            steps += 1;
            assert!(steps < 100);
        }
        assert_eq!(s.host.console.screen.fb[0], 9);
    }

    #[test]
    fn faults_name_the_array_and_line() {
        let mut s = session("var a[2]\nvar i = 5\ndraw() {\n  cls(a[i])\n}");
        assert_eq!(finish(&mut s, Handler::Draw), Step::Fault);
        assert_eq!(s.fault_line(), 4);
        assert_eq!(s.message(), "index out of bounds (a[5], size 2)");
    }

    #[test]
    fn audio_faults_are_reported_once() {
        let mut s = session("audio(t) {\n  var z = 0\n  return t / z\n}");
        assert!(s.has_audio());
        assert_eq!(s.audio_sample(1), (128, true));
        assert_eq!(s.audio_sample(2), (128, false));
        assert_eq!(s.message(), "division by zero");
    }

    #[test]
    fn bad_images_are_rejected() {
        assert!(Session::load(vec![1, 2, 3], true, 1, TestShelf::default()).is_err());
    }
}
