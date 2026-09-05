# Recording format 8

Format 8 retains all framing, fields, lifecycle, timing, checksums, directory,
and integrity rules from [format 7](recording-v7.md), with one explicit change:
`sampling_interval` accepts either `{"iterations": positive_u64}` or
`{"initial_and_final": true}`. No other keys are accepted.

Boundary sampling records the state after successful initialization and the
successful terminal state, once each, with equal iterations deduplicated.
Intermediate states, including u64::MAX, are not selected. Failed members have
no successful terminal checkpoint. Periodic sampling and boundary sampling are
mutually exclusive; the last builder call wins.

Workflow writes version 8 only when a recording contains a boundary-only stream;
otherwise it continues writing version 7. Rust 0.13.5 and Python 0.4.3 read both.
A version-7 document containing the new policy is invalid. Older readers reject
version 8 rather than misinterpreting its cadence. See the normative
[JSON schema](recording-v8.schema.json). NPY output remains format 2 and retains
the source recording's actual version.
