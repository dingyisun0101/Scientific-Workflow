//! Orchestrates the complete two-dimensional attractor workflow.
//!
//! The executable deliberately keeps implementation details in stage-specific
//! modules. This entry point sequences validated configuration, live state
//! evolution, bounded persistent recording, typed reconstruction, numerical
//! analysis, and explicit round-trip verification.

mod hopf_model;
mod project_setup;
mod recording_analysis;
mod state_recording;

use std::error::Error;
use std::path::PathBuf;
use std::process;

use project_setup::{ProjectSetup, create_execution_directory, load_project, prepare_task};
use recording_analysis::analyze_recording;
use state_recording::record_model;

/// Error boundary shared by the example's application modules.
///
/// Library and model-specific errors retain their concrete source chains while
/// the executable remains independent of an application error dependency.
pub(crate) type AppResult<T> = Result<T, Box<dyn Error>>;

/// Reports one terminal failure and selects an unsuccessful process status.
fn main() {
    if let Err(error) = run() {
        eprintln!("[error] {error}");
        process::exit(1);
    }
}

/// Sequences the full project without implementing any individual stage.
fn run() -> AppResult<()> {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ProjectSetup {
        project,
        schema,
        recording_root,
    } = load_project(&project_root)?;
    let task_count = project.parameters().task_count();
    let first = project.parameters().task(0)?;
    let model_name: String = first.decode_value("model_name")?;
    let step_count: u64 = first.decode_value("step_count")?;

    println!(
        "[project] model={} tasks={} step_count={} fields={} config={}",
        model_name,
        task_count,
        step_count,
        schema.len(),
        project.configuration_directory().display()
    );

    let execution_root = create_execution_directory(&recording_root)?;
    for parameters in project.parameters().tasks() {
        let (mut model, settings) = prepare_task(&schema, &parameters)?;
        println!(
            "[task] index={} mu={} omega={} physical_time_increment_per_step={}",
            settings.task_index,
            model.mu(),
            model.omega(),
            model.physical_time_increment_per_step()
        );

        let recording = record_model(&schema, &execution_root, &parameters, &settings, &mut model)?;
        let time = model.state().simulation_time();
        let point = model.point()?;
        println!(
            "[simulation] task={} final_iteration={} final_physical_time={} final_point=[{}, {}] final_radius={}",
            settings.task_index,
            time.iteration(),
            time.physical_time()
                .expect("the model tracks physical time"),
            point[0],
            point[1],
            model.radius()?
        );
        println!(
            "[storage] task={} recording={} streams=3 complete=true",
            settings.task_index,
            recording.directory.display()
        );

        let analysis = analyze_recording(&model, &recording)?;
        println!(
            "[analysis] task={} trajectory={} radius={} checkpoints={} x=[{}, {}] y=[{}, {}] radius=[{}, {}] final_radius={} expected_radius={}",
            settings.task_index,
            analysis.samples.trajectory,
            analysis.samples.radius,
            analysis.samples.checkpoint,
            analysis.x_bounds[0],
            analysis.x_bounds[1],
            analysis.y_bounds[0],
            analysis.y_bounds[1],
            analysis.radius_bounds[0],
            analysis.radius_bounds[1],
            analysis.final_radius,
            analysis.expected_attractor_radius
        );
        println!(
            "[plot] task={} legend=S:start,E:end,*:sample\n{}",
            settings.task_index, analysis.phase_portrait
        );
        println!("[verify] task={} round_trip=true", settings.task_index);
    }

    println!(
        "[result] attractor_2d=complete output_root={}",
        execution_root.display()
    );
    Ok(())
}
