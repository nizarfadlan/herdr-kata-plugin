build:
    cargo build --locked

test:
    cargo test --locked

check:
    cargo fmt --check
    cargo clippy --all-targets --locked -- -D warnings
    cargo test --locked

install-local: build
    mkdir -p bin
    cp target/debug/herdr-kata bin/herdr-kata
    chmod +x bin/run.sh scripts/*.sh
