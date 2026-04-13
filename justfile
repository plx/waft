build-release:
    cargo build --release

check-format:
    cargo fmt --check

check-clippy:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

check-test:
    cargo test --workspace --all-features

check-doc-test:
    cargo test --doc

check-doc-build:
    cargo doc --no-deps
