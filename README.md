# Scientific Workflow

## Running Tests

The Rust crate is located in `dev/`. Run test commands from that directory:

```bash
cd dev
```

### System-state value tests

The current test suite covers the private erased-value implementation in
`src/system_state/value.rs`. Its matching test file is
`tests/system_state/value.rs`.

Until the top-level Cargo integration-test harness is added, compile and run
this test target directly:

```bash
rustc \
  --edition 2024 \
  --test tests/system_state/value.rs \
  -o /tmp/scientific-workflow-value-tests

/tmp/scientific-workflow-value-tests
```

A successful run currently reports six passing tests:

```text
test result: ok. 6 passed; 0 failed
```

The direct `rustc` command is temporary. Cargo automatically discovers
integration-test entry points immediately inside `tests/`, but not test files
nested under `tests/system_state/`. Once `tests/system_state.rs` is created as
the suite entry point and the library facade is connected, the standard command
will be:

```bash
cargo test
```

Tests remain organized under subdirectories:

- A test covering one source file mirrors its source filename. For example,
  `src/system_state/value.rs` is tested by
  `tests/system_state/value.rs`.
- A test covering multiple source files uses a concise filename describing the
  behavior under test.
