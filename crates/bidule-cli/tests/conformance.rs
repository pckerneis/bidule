//! The Rust toolchain must draw what the JS browser prototype draws, frame for
//! frame (spec: Emulator and handheld conformance). Needs Node; skipped without it.

use std::path::{Path, PathBuf};
use std::process::Command;

use bidule_cli::{build, run_headless, AssetFiles, RunEnd};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn node(args: &[&str]) -> bool {
    Command::new("node")
        .current_dir(root())
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn check(cart: &str, frames: u32, inputs: &[u32]) {
    if !node(&["--version"]) {
        eprintln!("node not found; skipping {}", cart);
        return;
    }
    let dir = std::env::temp_dir().join(format!("bidule-conf-{}-{}", cart, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let assets = dir.join("assets");
    assert!(
        node(&["scripts/conformance.mjs", "assets", cart, assets.to_str().unwrap()]),
        "exporting assets"
    );
    let js = dir.join("js.fb");
    let list: Vec<String> = inputs.iter().map(|i| i.to_string()).collect();
    assert!(node(&[
        "scripts/conformance.mjs",
        "frames",
        cart,
        &frames.to_string(),
        js.to_str().unwrap(),
        &list.join(",")
    ]));
    let expect = std::fs::read(&js).unwrap();

    let src = std::fs::read_to_string(root().join("carts").join(format!("{}.bdcart", cart))).unwrap();
    let files = AssetFiles::read(&assets).unwrap();
    let (bytes, _) = build(&src, &files, true, cart).unwrap_or_else(|e| panic!("{}", e));
    let mut got = Vec::new();
    let end = run_headless(&bytes, true, frames, inputs, |_, con| got.extend_from_slice(&con.screen.fb)).unwrap();
    assert!(matches!(end, RunEnd::Frames), "{:?}", end);
    assert_eq!(got.len(), expect.len());
    for f in 0..frames as usize {
        let n = 240 * 160;
        let (a, b) = (&got[f * n..(f + 1) * n], &expect[f * n..(f + 1) * n]);
        if a != b {
            let i = a.iter().zip(b).position(|(x, y)| x != y).unwrap();
            panic!(
                "{}: frame {} differs first at ({}, {}): rust {} js {}",
                cart,
                f,
                i % 240,
                i / 240,
                a[i],
                b[i]
            );
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

const BTN_RIGHT: u32 = 2;
const BTN_DOWN: u32 = 8;
const BTN_A: u32 = 16;
const BTN_B: u32 = 32;

fn script(parts: &[(u32, usize)]) -> Vec<u32> {
    parts.iter().flat_map(|&(b, n)| std::iter::repeat_n(b, n)).collect()
}

#[test]
fn blink() {
    check(
        "blink",
        90,
        &script(&[(0, 10), (BTN_A, 1), (0, 20), (BTN_A, 1), (0, 5), (BTN_A, 1)]),
    );
}

#[test]
fn spark_catcher_play() {
    let s = script(&[
        (0, 3),
        (BTN_A, 1),
        (0, 1),
        (BTN_RIGHT, 40),
        (BTN_RIGHT | BTN_A, 20),
        (BTN_DOWN, 40),
        (BTN_A, 1),
        (0, 30),
    ]);
    check("spark-catcher", s.len() as u32, &s);
}

#[test]
fn tile_peek_cancelled_pick() {
    check("tile-peek", 30, &script(&[(0, 2), (BTN_A, 1), (0, 5), (BTN_B, 1), (0, 21)]));
}

#[test]
fn sprite_editor_cancelled_pick() {
    check(
        "sprite-editor",
        40,
        &script(&[
            (0, 2),
            (BTN_A, 1),
            (0, 3),
            (BTN_B, 1),
            (0, 2),
            (BTN_RIGHT, 4),
            (BTN_A | BTN_B, 1),
            (0, 26),
        ]),
    );
}
