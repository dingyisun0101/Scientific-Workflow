use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use scientific_workflow::prelude::*;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "scientific-workflow-artifact-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn exact_bytes_are_atomically_reused_and_verified() {
    let root = TempDirectory::new();
    let scope = ExecutionScope::create_named(&root.0, "execution").unwrap();
    let first = persist_artifact(&scope, "initial-space", "json", b"exact bytes").unwrap();
    let second = persist_artifact(&scope, "initial-space", "json", b"exact bytes").unwrap();

    assert_eq!(first.disposition(), ArtifactDisposition::Created);
    assert_eq!(second.disposition(), ArtifactDisposition::Reused);
    assert_eq!(first.descriptor(), second.descriptor());
    assert_eq!(
        load_verified_artifact(scope.directory(), first.descriptor())
            .unwrap()
            .bytes(),
        b"exact bytes"
    );
}

#[test]
fn altered_bytes_fail_digest_verification() {
    let root = TempDirectory::new();
    let scope = ExecutionScope::create_named(&root.0, "execution").unwrap();
    let persisted = persist_artifact(&scope, "input", "bin", b"original").unwrap();
    fs::write(
        scope.directory().join(persisted.descriptor().path()),
        b"altered",
    )
    .unwrap();

    assert!(matches!(
        load_verified_artifact(scope.directory(), persisted.descriptor()),
        Err(ArtifactLoadError::DigestMismatch { .. })
    ));
}

#[test]
fn unsafe_names_and_descriptor_paths_are_rejected() {
    let root = TempDirectory::new();
    let scope = ExecutionScope::create_named(&root.0, "execution").unwrap();
    assert!(matches!(
        persist_artifact(&scope, "../escape", "json", b"bytes"),
        Err(ArtifactError::InvalidFragment { .. })
    ));

    let descriptor: ArtifactDescriptor = serde_json::from_value(serde_json::json!({
        "sha256": "0".repeat(64),
        "path": "../outside"
    }))
    .unwrap();
    assert!(matches!(
        load_verified_artifact(scope.directory(), &descriptor),
        Err(ArtifactLoadError::InvalidDescriptor { .. })
    ));
}
