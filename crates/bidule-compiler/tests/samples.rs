use bidule_compiler::{compile, Assets, Options};

fn build(name: &str, src: &str) -> Vec<u8> {
    match compile(src, &Assets::default(), &Options::default()) {
        Ok(o) => {
            for w in &o.warnings {
                eprintln!("{}: {}", name, w);
            }
            o.cart
        }
        Err(ds) => {
            for d in &ds {
                eprintln!("{}: {}", name, d);
            }
            panic!("{} failed to compile", name);
        }
    }
}

#[test]
fn blink() {
    build("blink", include_str!("../../../carts/blink.bdcart"));
}
#[test]
fn new_cart() {
    build("new-cart", include_str!("../../../carts/new-cart.bdcart"));
}
#[test]
fn tile_peek() {
    build("tile-peek", include_str!("../../../carts/tile-peek.bdcart"));
}
#[test]
fn sprite_editor() {
    build("sprite-editor", include_str!("../../../carts/sprite-editor.bdcart"));
}
