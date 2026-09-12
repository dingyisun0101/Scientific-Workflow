# Upstream dependency review

Reviewed 2026-09-12 against the manifests, lockfile, upstream documentation,
and the crates.io/PyPI registry APIs. Versions below are the latest published
stable versions observed during this review, except explicitly named prereleases.
Release dates are maintenance evidence, not a guarantee of quality or support.
The user approved the proposed package list during the final decision review.
The release refresh uses synchronous fs4 1.1.0 and the latest compatible
registry versions. The macro crate source and manifest remain unchanged.

## Approved package choices

Retain the existing packages except replace `fs2` with `fs4` for filesystem
statistics and directory locking. Use only synchronous features. Do not add a
CLI framework, a host telemetry framework, or an asynchronous runtime for this
pass: the requested changes fit the existing runtime and terminal architecture.

| Direct Rust upstream | Latest observed; release date | Purpose, alternatives, and decision |
| --- | --- | --- |
| [crossterm](https://github.com/crossterm-rs/crossterm) | 0.29.0; 2025-04-05 | Retain for terminal mode, input, and restoration. Compared with [termion](https://docs.rs/termion/latest/termion/), it fits the existing Ratatui backend and offers cross-platform terminal support. Backend replacement provides no needed capability here. |
| [ratatui](https://ratatui.rs/) | 0.30.2; 2026-06-19 | Retain for the immediate-mode dashboard. Compared with [Cursive](https://github.com/gyscos/cursive), its layout and widget model already matches event-reduced state and a single refresh loop; a callback/widget-tree migration would add integration work without improving paging or thread display. |
| [serde](https://serde.rs/) | 1.0.229; 2026-07-18 | Retain for typed configuration and wire-format serialization. [borsh](https://docs.rs/borsh/latest/borsh/) and [rkyv](https://docs.rs/rkyv/latest/rkyv/) target different binary representations and do not replace the existing interoperable JSON contract. |
| [serde_json](https://github.com/serde-rs/json) | 1.0.151; 2026-07-20 | Retain for strict JSON parsing and round-trip floats, raw values, and declaration ordering. [simd-json](https://docs.rs/simd-json/latest/simd_json/) and [sonic-rs](https://docs.rs/sonic-rs/latest/sonic_rs/) are performance alternatives; adopting them needs measured parsing benefit and proof that duplicate-key and numeric behavior remain compatible. |
| [erased-serde](https://docs.rs/erased-serde/latest/erased_serde/) | 0.4.10; 2026-03-02 | Retain for object-safe serialization of heterogeneous scientific payloads. [typetag](https://github.com/dtolnay/typetag) adds tagged trait-object registration/deserialization above erased-serde, while Workflow already owns its decoder registry and schema identity. |
| [inventory](https://docs.rs/inventory/latest/inventory/) | 0.3.24; 2026-03-30 | Retain for linked execution-unit registration. [linkme](https://github.com/dtolnay/linkme) is a credible distributed-slice alternative, but changing the published registration macro and linker contract requires a separate demonstrated need. |
| [rayon](https://docs.rs/rayon/latest/rayon/) | 1.12.0; 2026-04-14 | Retain for task-private CPU pools and work stealing. Scoped standard threads require custom scheduling; [Tokio](https://tokio.rs/) addresses asynchronous I/O rather than the synchronous CPU parallelism exposed to execution units. |
| [fs2](https://docs.rs/fs2/latest/fs2/) → [fs4](https://docs.rs/fs4/latest/fs4/) | fs2 0.4.3; 2018-01-06. fs4 1.1.0; 2026-04-28 | Replace fs2: its last published release is much older, while fs4 maintains the relevant filesystem-statistics and locking functionality. [std::fs::File](https://doc.rust-lang.org/std/fs/struct.File.html) now provides locking but not filesystem capacity queries; [rustix](https://docs.rs/rustix/latest/rustix/) is another strong lower-level option. fs4 supplies both required capabilities with a small migration. Validate directory locking, contention, and one-snapshot capacity sampling before acceptance. |
| [libc](https://github.com/rust-lang/libc) | 0.2.189; 2026-07-21 | Retain for current Unix process-group control. rustix and [nix](https://docs.rs/nix/latest/nix/) provide safer wrappers, but replacing the small existing platform layer is not necessary for this pass. Keep unsafe calls contained and reviewed. |
| [sha2](https://github.com/RustCrypto/hashes) | 0.11.0; 2026-03-25 | Retain SHA-256 because it is part of persisted checksum compatibility and matches Python hashlib. [BLAKE3](https://github.com/BLAKE3-team/BLAKE3) is a credible faster-hash alternative, but adopting it would change the recording protocol; [ring](https://docs.rs/ring/latest/ring/) introduces a broader cryptography implementation than needed. |
| [thiserror](https://github.com/dtolnay/thiserror) | 2.0.20; 2026-08-08 | Retain typed subsystem errors and source chaining. [anyhow](https://github.com/dtolnay/anyhow) fits application-level opaque errors, while [SNAFU](https://docs.rs/snafu/latest/snafu/) would require a different error/context pattern. Neither improves the supported typed error API here. |
| [unicode-width](https://github.com/unicode-rs/unicode-width) | 0.2.2; 2025-10-06 | Retain terminal cell-width accounting. [unicode-segmentation](https://docs.rs/unicode-segmentation/latest/unicode_segmentation/) solves grapheme segmentation, a related but different problem; byte or character counts do not determine terminal width. |
| [time](https://github.com/time-rs/time) | 0.3.55; 2026-08-01 | Retain for UTC formatting already used by persistence. [Chrono](https://github.com/chronotope/chrono) and [Jiff](https://github.com/BurntSushi/jiff) are maintained alternatives, but this pass needs neither a second date/time type system nor timezone-database scheduling. |
| [scientific-workflow-macros](https://crates.io/crates/scientific-workflow-macros) | 0.2.1; 2026-08-28 | Retain the published upstream implementing Workflow's registration contract. Generic derive or registration packages do not replace this project-specific API. Do not edit this upstream crate without separate explicit permission. |
| [physics_in_parallel](https://crates.io/crates/physics_in_parallel) (development only) | 4.1.0-alpha; 2026-09-05 | Retain the published integration fixture. [ndarray](https://docs.rs/ndarray/latest/ndarray/) and [nalgebra](https://nalgebra.rs/) supply numerical arrays/algebra, not the PiP downstream compatibility contract exercised by these tests. This is a project-specific test dependency, not a general recommendation over those libraries. |

## Macro upstream inventory

These dependencies belong to the separately published macro crate. Their review
does not authorize editing that upstream. Keep the online macro release while
recording the available upgrades for a separately authorized pass.

| Package | Latest observed; release date | Rationale and alternatives |
| --- | --- | --- |
| [proc-macro2](https://github.com/dtolnay/proc-macro2) | 1.0.107; 2026-07-19 | Token streams usable outside procedural-macro invocation. Standard `proc_macro` is the lower-level alternative but is less convenient for ordinary library tests and the existing Syn/Quote stack. |
| [quote](https://github.com/dtolnay/quote) | 1.0.47; 2026-07-19 | Structured token generation integrated with Syn. Manual token construction or [genco](https://docs.rs/genco/latest/genco/) adds maintenance or a broader code-generation model without a requirement here. |
| [syn](https://github.com/dtolnay/syn) | 3.0.5; 2026-09-04; current manifest uses 2.x | Full Rust syntax parsing for implementation blocks. Handwritten token parsing is more fragile for this grammar; [darling](https://docs.rs/darling/latest/darling/) helps derive/attribute decoding but builds on Syn and is not a full replacement. Moving to Syn 3 is a major-version upstream change and needs separate authorization and compatibility validation. |

## Python upstream inventory

| Package | Latest observed; release date | Rationale and alternatives |
| --- | --- | --- |
| [NumPy](https://numpy.org/) | 2.5.3; 2026-09-06 | Retain as the optional NPY conversion implementation and canonical array ecosystem. [Zarr](https://zarr.dev/) and [h5py](https://www.h5py.org/) address different storage formats, while [PyArrow](https://arrow.apache.org/docs/python/) targets columnar interoperability. None replaces the requested NPY contract. |
| [threadpoolctl](https://github.com/joblib/threadpoolctl) | 3.6.0; 2025-03-13 | Retain for constraining native BLAS/OpenMP pools inside conversion workers. Environment variables alone may be applied too late or miss loaded libraries; [joblib](https://joblib.readthedocs.io/) adds a scheduler above the existing standard-process executor. NumPy itself documents threadpoolctl for native-pool control. |
| [setuptools](https://setuptools.pypa.io/) (build only) | 84.0.0; 2026-08-08 | Retain the established package-data and entry-point build backend. [Hatchling](https://hatch.pypa.io/latest/) and [Flit](https://flit.pypa.io/) are credible alternatives, but there is no packaging requirement justifying migration. |

Python's standard library supplies process scheduling, hashing, JSON, and CLI
parsing. No additional scheduling dependency is proposed. Use the latest
approved packages in an isolated Python 3.14 validation environment.

## Transitive dependencies and update policy

`Cargo.lock` is the full resolved transitive inventory. Transitives are selected
by their direct owners rather than independently substituted: Rayon owns its
worker deque/core stack; Ratatui/Crossterm own terminal backends and layout;
Serde/Thiserror own their derive implementations; SHA-2 owns its digest stack;
fs4 owns its platform filesystem bindings. Refresh compatible transitive
versions after approval, inspect the lockfile diff, and validate the resulting
graph. Do not force incompatible major upgrades through overrides.

The private examples intentionally consume the published Workflow package and
can retain older transitive versions through that package's requirements until
the new Workflow release is online. Update those consumers after publication.
No public upstream is consumed through a local checkout or patch override.

Registry evidence is directly reproducible at
`https://crates.io/api/v1/crates/<package>` and
`https://pypi.org/pypi/<package>/json`; each package's linked upstream supplies
the API and architectural evidence for the comparisons above.

## Approved refresh validation

Registry versions were rechecked against crates.io and PyPI for the approved
refresh. All reviewed direct versions still match the tables above. The
lockfile refresh advances compatible transitive dependencies. fs4 replaces
fs2 in Workflow; the older published Workflow used by private examples still
brings fs2 transitively until those consumers move to the online new release.
Python validation uses NumPy 2.5.3, threadpoolctl 3.6.0, and setuptools 84.0.0;
existing compatible requirement ranges remain unchanged.
