#[cfg(not(target_arch = "wasm32"))]
fn main() {
    market_ui::run_desktop();
}

#[cfg(target_arch = "wasm32")]
fn main() {}
