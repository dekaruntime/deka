// A plain cargo test must never silently skip the browser and parity gates.
#[test]
fn browser_and_tour_gates_are_enabled() {
    #[cfg(not(all(feature = "web", feature = "tour")))]
    panic!("Rust UI gates require --features tour,web; run scripts/test-rust-ui.sh");
}
