# Output layout

Each invocation creates `output/execution-YYYYMMDDTHHMMSS.nnnnnnnnnZ/`
under the project root. A collision adds a numeric suffix. This directory holds
`log.txt`, captured configuration and lifecycle data, and replicate directories.
Task ordinals are stable planned identities, not a counter of completion order.

```text
output/
└── execution-.../
    ├── log.txt
    └── replicate-000000/
        ├── task-000000/           single-member recording
        ├── task-000001/
        │   └── members/          multi-member recording directories
        └── task-000002/
            └── artifacts/        external task's domain outputs
```

This is a schematic, not a complete manifest. A task's kind determines its
contents. Raw recording metadata and chunks are owned by Workflow and should
be opened through verified readers. Program tasks own their domain artifacts;
the standard converter publishes manifests for its member datasets and batch.

Inside a pipeline, acquire paths through Rust or Python dependency selectors.
Outside a pipeline, select the actual completed execution before inspecting a
member. Do not infer completion from the presence of a directory.

See [Persistence](../../rust/src/persistence/api.md) for exact layout,
[NPY v2](../../protocol/npy-v2.md) for conversion manifests, and
[running and monitoring](../guide/10-running-and-monitoring.md) for logs and cleanup.
