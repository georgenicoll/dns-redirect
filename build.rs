use std::process::Command;

fn main() {
    // Run cargo fmt
    let _ = Command::new("cargo").args(["fmt", "--check"]).status();

    // Run cargo clippy
    let _ = Command::new("cargo")
        .args(["clippy", "--", "-D", "warnings"])
        .status();
}
