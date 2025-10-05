# dns-redirect

## Introduction

`dns-redirect` is a lightweight DNS server written in Rust that performs domain name redirection using CNAME records based on configurable regex pattern matching. The server intercepts DNS queries (A, AAAA, or ANY record types), applies regex-based transformations to the queried domain names, and responds with CNAME records pointing to the transformed domains. This is useful for local network setups, development environments, or scenarios where you need to dynamically redirect domain queries without modifying DNS zone files.

## Building and Running

### Prerequisites

- Rust toolchain (edition 2024)
- Cargo package manager

### Build for Native Target

To build the project for your current platform:

```bash
cargo build --release
```

The compiled binary will be located at `target/release/dns-redirect`.

Alternatively, use the provided Makefile which includes formatting and linting:

```bash
make build
```

### Cross-Compile for ARM64

To cross-compile for ARM64 (aarch64) Linux systems:

1. Install the ARM64 target and cross-compiler:

```bash
rustup target add aarch64-unknown-linux-gnu
sudo apt install gcc-aarch64-linux-gnu
```

2. Build for ARM64:

```bash
cargo build --release --target aarch64-unknown-linux-gnu
```

Or using the Makefile:

```bash
make arm64
```

The ARM64 binary will be located at `target/aarch64-unknown-linux-gnu/release/dns-redirect`.

### Running the Server

Run the server with the default configuration file (`config.json`):

```bash
./target/release/dns-redirect
```

Or specify a custom configuration file:

```bash
./target/release/dns-redirect --config-file /path/to/config.json
```

## How It Works

### Architecture

The project implements a custom DNS server using the `hickory-server` library (formerly Trust-DNS). The server operates asynchronously using Tokio and handles UDP DNS queries on a configurable bind address.

### Core Components

**Configuration (`Config`)**
- Loaded from a JSON file specifying the bind address and regex replacement rules
- Each replacement rule consists of a regex pattern (`from`) and a replacement template (`to`)

**Request Handler (`DomainConversionHandler`)**
- Implements the `RequestHandler` trait from hickory-server
- Processes incoming DNS queries by matching the queried domain against configured regex patterns
- Returns CNAME records for matches or NXDomain responses for non-matches

**Pattern Matching**
- Uses Rust's `regex` crate for pattern matching
- Supports capture groups in regex patterns (e.g., `^(.*)\\.internal-net(\\.?)$`)
- Replacement templates use `{N}` syntax to reference capture groups (e.g., `{1}.lan{2}`)

### Request Flow

1. DNS query arrives at the server (UDP socket)
2. Server extracts the queried domain name and record type
3. If the query is for A, AAAA, or ANY records:
   - The domain is matched against each regex pattern in order
   - First matching pattern triggers a transformation
   - Server responds with a CNAME record pointing to the transformed domain
4. If no pattern matches or the query is for unsupported record types:
   - Server responds with NXDomain (domain not found)

### Configuration Format

The `config.json` file structure:

```json
{
    "bind_address": "127.0.0.1:8053",
    "replacements": [
        {
            "from": "^(.*)\\.internal-net(\\.?)$",
            "to": "{1}.lan{2}"
        }
    ]
}
```

- `bind_address`: IP address and port for the DNS server to listen on
- `replacements`: Array of regex transformation rules applied in order
- `from`: Regex pattern to match against queried domains
- `to`: Template string with `{N}` placeholders for capture groups

### Dependencies

**Core Dependencies:**
- `hickory-server` (0.25.2) - DNS server implementation
- `hickory-proto` (0.25.2) - DNS protocol types and utilities
- `tokio` (1.47.1) - Async runtime with multi-threaded support
- `async-trait` (0.1.89) - Async trait support
- `regex` (1.x) - Regular expression matching
- `serde` (1.0.228) - Serialization/deserialization framework
- `serde_json` (1.0.137) - JSON parsing for configuration
- `strfmt` (0.2.5) - String formatting with named placeholders
- `clap` (4.5.48) - Command-line argument parsing
- `anyhow` (1.0.100) - Error handling

**Development Dependencies:**
- `hickory-resolver` (0.25.2) - DNS resolver for testing
- `futures` (0.3.28) - Async utilities for tests
- `tokio` (test-util feature) - Testing utilities

### Example Use Cases

**Local Development Environment:**
Redirect `*.internal-net` to `*.lan` for local network access:
```json
{"from": "^(.*)\\.internal-net(\\.?)$", "to": "{1}.lan{2}"}
```

**Multi-level Domain Transformation:**
Swap subdomain components `x.y.pod` → `y.x.pod`:
```json
{"from": "^(.*)\\.(.*)\\.pod\\.?$", "to": "{2}.{1}.pod."}
```

**Wildcard Redirection:**
Redirect all queries to a single domain:
```json
{"from": "^.*$", "to": "fallback.example.com."}
```
