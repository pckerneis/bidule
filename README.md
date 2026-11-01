# bidule-web

Browser emulator and default apps for **Bidule 01**, an open fantasy console built around easy-to-source hardware (240×160 indexed screen, D-pad, A/B).

The emulator runs `.bdcart` source directly, following the [Bidule 01 technical specification](spec/technical-specification.md): the cart language (int and 16.16 `fix`, records, enums, constant tables, string interpolation), the asset sections (sprite sheet, maps, sounds, palette), the editing API and the system dialogs.

The repository also holds the Rust toolchain that will replace that interpreter everywhere: the compiler, the bytecode verifier, the VM and the console runtime, all `no_std`, following the [bytecode and cart image format](spec/bytecode.md).

## Run it

```sh
npm run build        # writes dist/index.html
```

Open `dist/index.html` in a browser. There is nothing to install for the page itself: everything is inlined in one file.

Keys: arrows or WASD for the D-pad, X/K for A, Z/J for B, R to reset, M for sound. The mouse works on the screen as the optional pointer.

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

Tests cover the language end to end, the format, hostile carts (mutated images and random bytecode must be rejected or run safely), pixel-for-pixel conformance with the JS emulator on every sample cart, and the compiler's memory use.

## Layout

| Path | What it holds |
| --- | --- |
| `spec/` | The specifications: technical spec, bytecode format, design document, threat model |
| `crates/` | The Rust toolchain (see above) |
| `src/js/` | The console, one module per concern, concatenated in the order listed in `scripts/build.mjs` |
| `src/shell.html`, `src/styles.css` | The page around the screen: device, cart shelf, code editor |
| `carts/` | Sample carts and default apps as `.bdcart` source; their asset data lives in `src/js/samples.js` |
| `fonts/monogram/` | Monogram bitmap (CC0) and its credits; the build turns it into the console font |
| `scripts/build.mjs` | Builds `dist/index.html` (standalone) and `dist/artifact.html` (body fragment) |
| `scripts/conformance.mjs` | Exports sample assets and dumps JS framebuffers for the Rust conformance tests |
| `tests/` | Node tests: language, assets and editing API, a scripted Sprite Editor session, the page in a simulated browser |

### Modules

| Module | Role |
| --- | --- |
| `carts.js` | Cart model, asset sections, (de)serialization |
| `samples.js` | Sample carts with their sprites, maps and sounds |
| `palette.js`, `font.js`, `framebuffer.js` | 256-entry RGB565 palette, Monogram 6×9 font, 8-bit framebuffer and primitives |
| `audio.js` | 4 synth voices for `Sound` values, bytebeat `audio(t)` |
| `lexer.js`, `parser.js`, `values.js` | Source → AST, runtime value types |
| `interpreter.js` | Resumable interpreter (generators), built-ins, asset names, editing API |
| `host.js` | Shelf, main loop, system dialogs, input, code editor |

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

- The browser emulator still interprets source: the Rust compiler and VM aren't wired into the page yet (next: a WebAssembly build). Until then, compile-time errors in the browser appear when a line first runs.
- The Rust compiler doesn't meet the spec's 128 KB working-RAM budget yet (about 700 KB for the Sprite Editor). It doesn't compile constant `Sound` records in source, music or byte strings, and the Rust runtime's editing API stops at `pickcart`, `savecart` and `status`.
- In the browser, `rnd` uses the browser's generator, so sequences are not reproducible. The Rust VM uses a seeded xorshift32.
- Music assets and slot emptying are not implemented; carts can't be exported or imported as files yet.

## Credits

Monogram is a free, Creative Commons Zero pixel font by Vinícius Menézio ([datagoblin.itch.io/monogram](https://datagoblin.itch.io/monogram)). See `fonts/monogram/CREDITS.txt`.
