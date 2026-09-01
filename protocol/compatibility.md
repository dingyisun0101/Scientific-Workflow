# Protocol Compatibility

The machine-readable authority for this table is
[`compatibility.json`](compatibility.json). Recording-format versions and
package versions are independent. A package supports only the versions listed;
unknown recording versions fail closed.

| Implementation | Package version | Writes | Reads |
| --- | ---: | ---: | ---: |
| Rust `scientific-workflow` | 0.13.2 | v7 | v7 |
| Python `scientific-workflow-reader` | 0.4.1 | — | v7 |

The Python package has no supported writer API. Its test-only round-trip bridge
is conformance infrastructure, not a protocol producer offered to users.

Rust 0.13.2 accepts project configuration `workflow_schema: 1`. That authored
configuration generation is separate from recording format v7: changing one
does not implicitly change the other.

This matrix was reviewed for Rust 0.13.2 with Python reader 0.4.1. Recording
format v7 is unchanged by that release.

For the normative recording contract and coordinated bump policy, see
[`recording-v7.md`](recording-v7.md).
