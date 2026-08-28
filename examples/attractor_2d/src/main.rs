//! A complete scientific-model-plus-JSON Workflow project.

mod hopf_model;

/// Hands the project root to Workflow's complete load, validate, and run path.
///
/// The registration attribute in `hopf_model` makes `HopfModel` discoverable;
/// no model construction or scheduling is needed in this application entry.
fn main() -> Result<(), scientific_workflow::WorkflowError> {
    scientific_workflow::run(std::path::Path::new(env!("CARGO_MANIFEST_DIR")))
}
