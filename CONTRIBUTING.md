# Contributing to csvdb

## Building

```bash
# Core CLI
cargo build --release

# Run it
cargo run --release -- to-csvdb mydb.sqlite
```

## Testing

```bash
# Rust unit tests
cargo test --release -p csvdb

# FFI tests
cargo test --release -p csvdb-ffi

# Python functional tests (189+ tests)
cd tests/functional && uv run pytest

# Python binding tests
cd csvdb-python && uv run maturin develop --release && uv run pytest

# Perl tests
cargo build --release -p csvdb-ffi
prove perl/t/
```

All tests must pass on Linux, macOS, and Windows. CI runs automatically on PRs.

## Project Structure

```
csvdb/           Core Rust library + CLI binary
csvdb-python/    Python bindings (PyO3)
csvdb-ffi/       C FFI shared library
perl/            Perl module (FFI::Platypus)
tests/functional Python integration tests
```

## Adding a New Command

1. Create `csvdb/src/commands/your_command.rs`
2. Add `pub mod your_command;` to `csvdb/src/commands/mod.rs`
3. Wire it into the CLI in `csvdb/src/main.rs` (add clap subcommand + match arm)
4. Add functional tests in `tests/functional/test_your_command.py`
5. Expose via bindings if appropriate (`csvdb-python/src/lib.rs`, `csvdb-ffi/src/lib.rs`)

Follow the pattern of existing commands like `checksum.rs` or `sql.rs`.

## Pull Requests

- Include tests for new functionality
- CI must pass on all platforms
- Keep PRs focused — one feature or fix per PR
- Update README if adding user-facing features

## Reporting Issues

Use the [issue templates](https://github.com/jeff-gorelick/csvdb/issues/new/choose) for bug reports and feature requests.
