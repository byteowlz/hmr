# hmr - Home Assistant CLI
# https://github.com/byteowlz/hmr

set positional-arguments

# === Default ===

# List available commands
default:
    @just --list

# === Build ===

# Build debug binary
build:
    cargo build

# Build release binary
build-release:
    cargo build --release

# Fast compile check
check:
    cargo check

# === Test ===

# Run tests
test:
    cargo test

# Run tests with all features
test-all:
    cargo test --all-features

# === Lint & Format ===

# Run clippy linter
clippy:
    cargo clippy -- -D warnings

# Alias for clippy
lint: clippy

# Auto-fix lint warnings
fix:
    cargo clippy --fix --allow-dirty

# Format code
fmt:
    cargo fmt

# Check formatting
fmt-check:
    cargo fmt -- --check

# === Install ===

# Install to ~/.cargo/bin
install:
    cargo install --path .

# Install release build
install-release:
    cargo install --path . --release

# === Docs ===

# Generate documentation
docs:
    cargo doc --no-deps --open

# === Clean ===

# Clean build artifacts
clean:
    cargo clean

# === Development ===

# Run in development mode
run *args:
    cargo run -- {{args}}

# Watch for changes and rebuild
watch:
    cargo watch -x check

# === Release ===

# Update dependencies
update:
    cargo update
