# Program events version 1

Transport: newline-terminated UTF-8 on stderr, prefix `@workflow ` followed by one
JSON object. Every original byte remains in stderr.log. Maximum full frame size:
16 KiB including prefix/newline. Producers must flush each event. Unknown ordinary
output is valid. JSON routing identifiers are ignored: Runtime attaches task and
replicate identity from the actual child launch.

Required fields: `version: 1`, `kind: "log" | "progress"`.

Log: `level` is debug/info/warning/error/success; `message` is a string.
Progress: nonempty `stage` and `unit` strings; `completed` is u64; `total` is null,
absent, or u64 at least completed. Counts describe a stage, not predicted duration.
Additional fields are ignored for compatible extension. Unsupported versions/kinds,
malformed JSON, oversized frames and unterminated final frames produce bounded
warnings (at most three per task) while raw bytes remain available. Required-log
or pipe-read failures are fatal. Display strips control characters and limits
text to 2048 characters. Progress is coalesced at 50ms; the latest update is
flushed at stream completion. Lifecycle/outcome messages are not coalesced.

Python 0.4.3 exposes `scientific_workflow.reporting` with log/progress and an
opt-in standard logging Handler. Imports never configure root logging. Outside
Workflow these events remain readable stderr lines. Standard converter workers
send bounded internal updates to one parent emitter. This protocol version is
independent of package, recording, NPY and configuration versions.

The converter's pause/cancel control file and per-job acknowledgements are private,
coordinated implementation details, not a general external-program control API.
