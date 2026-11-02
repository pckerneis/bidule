# bidule-web

Browser emulator and default apps for **Bidule 01**, an open fantasy console built around easy-to-source hardware (240×160 indexed screen, D-pad, A/B).

The emulator compiles `.bdcart` source to bytecode and runs it, following the [Bidule 01 technical specification](spec/technical-specification.md): the cart language (int and 16.16 `fix`, records, enums, constant tables, string interpolation), the asset sections (sprite sheet, maps, sounds, palette), the editing API and the system dialogs.

The console itself is written in Rust: the compiler, the bytecode verifier, the VM and the runtime, following the [bytecode and cart image format](spec/bytecode.md). The page runs them as WebAssembly; the same `no_std` crates build for the hardware.

## Run it

```sh
rustup target add wasm32-unknown-unknown    # once
npm run build                               # writes dist/index.html
```

Open `dist/index.html` in a browser. There is nothing to install for the page itself: everything, WebAssembly included, is inlined in one file.

Keys: arrows or WASD for the D-pad, X/K for A, Z/J for B, R to reset, M for sound, F for full screen. The mouse works on the screen as the optional pointer.

On a phone or tablet, **Full screen** shows the screen in landscape with the D-pad and A/B drawn over it as a semi-transparent overlay: multi-touch, and sliding a finger from one button to another switches buttons. The **⇄** control at the top switches to mouse mode: the buttons go away and a finger on the screen is the pointer (the editors use it); in button mode touches on the screen are ignored. Where the browser allows it, the page also locks landscape and keeps the screen awake. iPhone Safari has no full-screen API for pages, so there the overlay fills the browser window instead.

## Rust toolchain

```sh
cargo run -p bidule-cli -- build carts/blink.bdcart           # writes carts/blink.bdb
cargo run -p bidule-cli -- run carts/blink.bdcart --frames 120 --ppm blink.ppm
cargo run -p bidule-cli -- dis carts/blink.bdcart             # disassembly
cargo test --workspace                                         # Node is needed for the conformance tests
```

`bidule build` and `bidule run` take asset sections from `--assets DIR` (`sprites.bin`, `maps.bin`, `sounds.bin`, `music.bin`, `palette.bin`), or from a folder named like the cart. `node scripts/conformance.mjs assets spark-catcher DIR` exports a sample cart's assets from the JS emulator in that layout.

| Crate | Role |
| --- | --- |
| `bidule-bytecode` | The format: opcodes, built-in ids, `.bdb` container and sections, array handles, verifier (`no_std`) |
| `bidule-compiler` | `.bdcart` source and asset sections to `.bdb` (`no_std` + `alloc`) |
| `bidule-vm` | Interpreter with fixed RAM, instruction budget, debug and release faults, native math and text (`no_std`, no allocation) |
| `bidule-runtime` | Renderer, Monogram font, input, sound requests, asset access: the system side of the API (`no_std`) |
| `bidule-cli` | `bidule build`, `run` (headless), `dis`, `check`, `unpack` |
| `bidule-web` | The compiler, VM and runtime as a plain WebAssembly interface for the page, with the editing API bridged to the page's cart shelf |

Tests cover the language end to end, the format, hostile carts (mutated images and random bytecode must be rejected or run safely), pixel-for-pixel conformance with the JS reference interpreter on every sample cart, natively and as WebAssembly, and the compiler's memory use.

## Layout

| Path | What it holds |
| --- | --- |
| `spec/` | The specifications: technical spec, bytecode format, design document, threat model |
| `crates/` | The Rust toolchain (see above) |
| `src/js/` | The page around the console, one module per concern, concatenated in the order listed in `scripts/build.mjs` |
| `src/shell.html`, `src/styles.css` | The page around the screen: device, cart shelf, code editor |
| `carts/` | Sample carts and default apps as `.bdcart` source; their asset data lives in `src/js/samples.js` |
| `fonts/monogram/` | Monogram bitmap (CC0) and its credits; the build turns it into the console font |
| `scripts/build.mjs` | Builds the WebAssembly, then `dist/index.html` (standalone) and `dist/artifact.html` (body fragment) |
| `scripts/conformance.mjs` | Exports sample assets and dumps the reference interpreter's framebuffers for the Rust conformance tests |
| `tests/` | Node tests, on the WebAssembly console: language, assets and editing API, a scripted Sprite Editor session, conformance, the page in a simulated browser |

### Modules

| Module | Role |
| --- | --- |
| `carts.js` | Cart model, asset sections, (de)serialization |
| `samples.js` | Sample carts with their sprites, maps and sounds |
| `palette.js`, `font.js`, `framebuffer.js` | 256-entry RGB565 palette, Monogram 6×9 font, 8-bit framebuffer and primitives, for system dialogs and the display |
| `audio.js` | 4 synth voices for `Sound` values, bytebeat `audio(t)` |
| `editing.js` | The shelf's side of the editing API: handles on picked carts' asset sections |
| `wasm.js` | Runs a cart on the WebAssembly console: compile, load, resumable tasks, assets in their on-disk layout |
| `host.js` | Shelf, main loop, system dialogs, input, code editor |
| `lexer.js`, `parser.js`, `values.js`, `interpreter.js` | Not in the page: the earlier source interpreter, kept as the reference for conformance tests |

## Default apps

- **Sprite Editor** (`carts/sprite-editor.bdcart`): edits the tile sheet of any cart on the shelf through the editing API. Tap B to move between panels; mouse works everywhere.
- **Tile Peek**: a minimal editor kept as an editing-API example.
- **Spark Catcher**, **Blink**: games using assets and the spec's example cart.

## Tests

```sh
npm install          # jsdom, for the page tests
npm test
```

## Known gaps

- The Rust compiler doesn't meet the spec's 128 KB working-RAM budget yet (about 700 KB for the Sprite Editor). It doesn't compile constant `Sound` records in source, music or byte strings: in the page these are load errors.
- `readasset` and `writeasset` move raw bytes, so they expect `int` arrays: a `fix` array doesn't get the converted values the reference interpreter gave.
- A running cart sees its assets as they were when it loaded. If an editor saves into the running cart, the changes show after a reset.
- Music assets and slot emptying are not implemented; carts can't be exported or imported as files yet.

## Credits

Monogram is a free, Creative Commons Zero pixel font by Vinícius Menézio ([datagoblin.itch.io/monogram](https://datagoblin.itch.io/monogram)). See `fonts/monogram/CREDITS.txt`.
