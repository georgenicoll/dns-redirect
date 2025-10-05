.PHONY: build check fmt clippy

build: fmt clippy
	cargo build --release

check: fmt clippy
	cargo check

fmt:
	cargo fmt

clippy:
	cargo clippy -- -D warnings

arm64: fmt clippy
	cargo build --release --target aarch64-unknown-linux-gnu