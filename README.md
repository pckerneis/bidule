# bidule-web

Browser emulator and default apps for **Bidule 01**, an open fantasy console built around easy-to-source hardware (240×160 indexed screen, D-pad, A/B).

The emulator runs `.bdcart` source directly, following the *Bidule 01 — Technical Specification*: the cart language (int and 16.16 `fix`, records, enums, constant tables, string interpolation), the asset sections (sprite sheet, maps, sounds, palette), the editing API and the system dialogs.

## Run it

```sh
npm run build        # writes dist/index.html
```

Open `dist/index.html` in a browser. There is nothing to install for the page itself: everything is inlined in one file.

Keys: arrows or WASD for the D-pad, X/K for A, Z/J for B, R to reset, M for sound. The mouse works on the screen as the optional pointer.

## Layout

| Path | What it holds |
| --- | --- |
| `src/js/` | The console, one module per concern, concatenated in the order listed in `scripts/build.mjs` |
| `src/shell.html`, `src/styles.css` | The page around the screen: device, cart shelf, code editor |
| `carts/` | Sample carts and default apps as `.bdcart` source; their asset data lives in `src/js/samples.js` |
| `scripts/build.mjs` | Builds `dist/index.html` (standalone) and `dist/artifact.html` (body fragment) |
| `tests/` | Node tests: language, assets and editing API, a scripted Sprite Editor session, the page in a simulated browser |

### Modules

| Module | Role |
| --- | --- |
| `carts.js` | Cart model, asset sections, (de)serialization |
| `samples.js` | Sample carts with their sprites, maps and sounds |
| `palette.js`, `font.js`, `framebuffer.js` | 256-entry RGB565 palette, 6×9 font, 8-bit framebuffer and primitives |
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

- No compiler, bytecode or verifier yet: source is interpreted, and compile-time errors appear when a line first runs.
- A stand-in font replaces Monogram until it is bundled.
- `rnd` uses the browser's generator, so sequences are not reproducible.
- Music assets and slot emptying are not implemented; carts can't be exported or imported as files yet.
