# Task input snapshots v1 and completion receipts v2

## Ownership

Workflow captures authored configuration once and gives each task resolved
immutable inputs. Persistence saves the input snapshots; Runtime verifies
completed-task identity and compatibility for orchestration. Scientific analysis
and historical result interpretation belong to downstream consumers.

## Task-owned files

Every completed source task directory contains:

- `workflow-config.json`: the centrally captured configuration snapshot;
- `workflow-dependencies.json`: the task's resolved dependency snapshot; and
- `workflow-inputs.json`: the immutable checksum manifest.

A reused task placeholder contains its new receipt referring to the original
source output directory and checksum references. It does not copy input snapshots.

The manifest has this shape:

```json
{
  "format": "scientific-workflow-inputs.v1",
  "config": {
    "path": "workflow-config.json",
    "checksum": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
  },
  "dependencies": {
    "path": "workflow-dependencies.json",
    "checksum": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
  }
}
```

The example digests illustrate syntax, not actual file contents. Paths MUST
identify the two canonical task-root files. Digests MUST be `sha256:` followed
by 64 lowercase hexadecimal characters and cover exact saved bytes.
Unknown fields, formats, malformed references, missing files, and checksum
mismatches fail verification. Mutable runtime-control JSON is not an input
snapshot and has no immutable snapshot checksum.

## Program metadata and receipts

Program metadata uses `scientific-workflow-program-v2`. Completed-task receipts
use `scientific-workflow-task-result.v2`. Both carry required `inputs` with
the same `config` and `dependencies` descriptors as the input manifest.
These authorities MUST agree with the manifest and original in-memory
references while a task is running. Snapshot files are never rewritten to make
verification succeed.

A receipt binds the completed task identity, global configuration, output
directory, and workload to those verified inputs. Receipt publication refuses
replacement. Reuse preserves the original task output and its checksum
references; new execution receipts do not manufacture a new source history.

## Verification and compatibility

Verify exact snapshot bytes before successful completion, supported input
reads, and reuse. Parse only the verified bytes for configuration/dependency
access. This protects integrity relative to the saved references; checksums do
not establish authorship or scientific correctness.

After integrity verification, compare work-relevant scientific inputs with the
new captured intent. Resource settings, scheduling, phase selection, and Python
environment selection retain their documented operational exceptions. A
whole-file checksum difference between two runs is not itself incompatibility.
Resolved ordinary executables and scientific Python scripts must match their
recorded identity; source-code contents are not hashed.

## Version boundary

Workflow-controlled reuse accepts only current v2 receipts/program metadata.
It does not import missing/legacy completion evidence or retroactively hash old
snapshots. Historical results remain application-owned inputs for downstream
readers. Raw recordings keep their separate v7/v8 structural contract.
