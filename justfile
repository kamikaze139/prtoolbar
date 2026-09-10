# Run all the checks CI runs.
check:
    cargo fmt --all --check
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --all-features

# Format the code.
fmt:
    cargo fmt --all

# Run the app with debug logging.
run:
    RUST_LOG=prtoolbar=debug cargo run

# Build prtoolbar.app into target/release/bundle (needs `cargo install cargo-bundle`).
bundle:
    cargo bundle --release

# Check licences and advisories (needs `cargo install cargo-deny`).
deny:
    cargo deny check
