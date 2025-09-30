# dns-redirect

A rust project to redirect a domain (or domains) using DNS cnames.

## Compilation

```bash
cargo build --release
```

## Cross-compile for arm64

```bash
cargo build --release --target aarch64-unknown-linux-gnu
```

this requires that the arm64 target and cross-compiler are installed:

```bash
rustup target add aarch64-unknown-linux-gnu
sudo apt install gcc-aarch64-linux-gnu
```
