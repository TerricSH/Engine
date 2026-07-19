//! Regenerate `tests/golden_vectors.json`.
//!
//! Run only after an intentional, `PROCGEN_SCHEMA`-bumping algorithm change:
//!
//! ```sh
//! cargo run -p engine-procgen --example generate_golden_vectors \
//!   > crates/engine-procgen/tests/golden_vectors.json
//! ```
//!
//! Then review the diff like any other code change — the vector diff *is* the
//! algorithm change.

fn main() {
    let vectors = engine_procgen::golden::generate();
    println!(
        "{}",
        serde_json::to_string_pretty(&vectors).expect("golden vectors serialize")
    );
}
