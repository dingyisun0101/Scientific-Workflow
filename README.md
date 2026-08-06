# Scientific Workflow

## Running Tests

The Rust crate is located in `dev/`. Run test commands from that directory:

```bash
cd dev
```

### Complete system-state suite

The Cargo integration entry point includes every focused suite under
`tests/system_state/` plus the public tensor-backed integration workflow:

```bash
cargo test --test system_state
```

Focused suites can be selected by module path:

```bash
cargo test --test system_state 'spec_tests::'
cargo test --test system_state 'error_tests::'
cargo test --test system_state 'value_tests::'
cargo test --test system_state 'state_tests::'
```

### Complete in-memory time-series suite

The time-series target includes the focused collection and error suites plus a
public SystemState-to-StateSeries ownership workflow:

```bash
cargo test --test time_series
```

Focused suites can be selected by module path:

```bash
cargo test --test time_series 'error_tests::'
cargo test --test time_series 'series_tests::'
```

Tests remain organized under subdirectories:

- A test covering one source file mirrors its source filename. For example,
  `src/system_state/value.rs` is tested by
  `tests/system_state/value.rs`.
- A test covering multiple source files uses a concise filename describing the
  behavior under test.
