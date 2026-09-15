# 2. Installation

**Outcome:** prepare an environment that can build the Rust application and run
the optional Python converter. You need Linux, a terminal, and Rust 1.97+.

## Check prerequisites

```sh
rustc --version
cargo --version
python3.14 --version
```

Use your system's Rust installation method to obtain Rust 1.97 or newer. Python
3.14+ is required for Workflow's Python companion and `$npy`; the Rust-only
first study does not require the companion. Generic external programs may have
their own interpreter requirements.

## Create or activate one Python environment

For a new environment, run from your application directory:

```sh
python3.14 -m venv .venv
source .venv/bin/activate
python -m pip install --upgrade pip
python -m pip install 'scientific-workflow[npy]==0.5.0'
```

If `venv` reports that `ensurepip` is unavailable, install your Linux
distribution’s Python 3.14 venv support and recreate the environment.

If you already have a suitable environment, activate it instead. Cargo does not
install Python packages, and Workflow does not create or activate environments.
Omit `[npy]` only when you need the core recording reader without conversion.

Verify the actual interpreter and imports:

```sh
python3 -c 'import sys, scientific_workflow, numpy, threadpoolctl; print(sys.executable, scientific_workflow.__version__)'
```

Expect your selected environment's interpreter and companion version `0.5.0`.
The standard converter resolves `python3` from the active `PATH`. Activate the
same environment before every launch, including in new shells and multiplexer
sessions. [Chapter 9](9-python-analysis-and-visualization.md) explains explicit
interpreter selection for ordinary Python tasks.

## Add Rust dependencies

Inside an existing Cargo application:

```sh
cargo add scientific-workflow@0.15.2
cargo add serde --features derive
```

The execution-unit registration macro is re-exported by Workflow. Applications
do not need a separate macro dependency. Commit an application's `Cargo.lock`
to retain its resolved dependency versions. The next chapter supplies a complete
manifest if you are starting from an empty directory.

## Prepare an interactive session

For a remote or long experiment:

```sh
tmux new -s workflow
```

Alternatively use `screen -S workflow`. Activate the Python environment inside
that session, then run your application. Standard input and standard error must
be terminals. Headless or redirected execution is rejected before output
creation or cleanup. Detach and reattach to the multiplexer to keep a run alive.

## Checkpoint

You should now have the required Rust version, a usable terminal, and—if you
intend to convert or analyze recordings—a verified Python environment. See
[troubleshooting](../troubleshooting.md) if any prerequisite check fails.

---

**Previous:** [1. Overview](1-overview.md) · **Guide index:** [Documentation](../README.md)

**Next:** [3. Your first study](3-first-study.md)
