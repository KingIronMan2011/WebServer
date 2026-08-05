# Fuzzing

Install the Rust nightly toolchain and cargo-fuzz once, then run a target:

```sh
cargo +nightly install cargo-fuzz
cd fuzz
cargo +nightly fuzz run config
cargo +nightly fuzz run http_path
```

Crash inputs are written below `fuzz/artifacts/`; add minimized regressions to the normal test suite before fixing them.
