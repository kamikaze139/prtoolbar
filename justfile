# Run all the checks CI runs.
check:
    cargo fmt --all --check
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --all-features
    python3 -m unittest discover -s scripts -p 'test_*.py'

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

# Scan the whole git history for committed secrets (needs `brew install gitleaks`).
scan-secrets:
    gitleaks git . --redact --no-banner --exit-code 1

# Build the universal, ad-hoc signed prtoolbar.app exactly as the release CI does.
bundle-universal:
    MACOSX_DEPLOYMENT_TARGET=12.0 cargo build --release --locked --target aarch64-apple-darwin
    MACOSX_DEPLOYMENT_TARGET=12.0 cargo build --release --locked --target x86_64-apple-darwin
    cargo bundle --release --format osx --target aarch64-apple-darwin
    lipo -create -output target/aarch64-apple-darwin/release/bundle/osx/prtoolbar.app/Contents/MacOS/prtoolbar target/aarch64-apple-darwin/release/prtoolbar target/x86_64-apple-darwin/release/prtoolbar
    codesign --force --options runtime --sign - target/aarch64-apple-darwin/release/bundle/osx/prtoolbar.app
    @echo "→ target/aarch64-apple-darwin/release/bundle/osx/prtoolbar.app"
