//! A complete execution-unit-plus-JSON Workflow project.

mod hopf_unit;

fn main() -> Result<(), scientific_workflow::WorkflowError> {
    scientific_workflow::run(std::path::Path::new(env!("CARGO_MANIFEST_DIR")))
}
