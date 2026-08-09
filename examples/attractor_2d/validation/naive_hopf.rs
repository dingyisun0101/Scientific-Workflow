//! Naive one-file reference for the `attractor_2d` scientific result.
//!
//! Compile this file directly with `rustc`. It intentionally has no project
//! configuration, heterogeneous state, recording, decoding, or
//! `scientific-workflow` dependency.

const INITIAL_POINT: [f64; 2] = [0.25, 0.0];
const ANGULAR_FREQUENCY: f64 = 1.0;
const TIME_STEP: f64 = 0.01;
const TOTAL_STEPS: u64 = 5_000;
const MU_VALUES: [f64; 3] = [-0.25, 0.25, 1.0];

fn main() {
    for (task, mu) in MU_VALUES.into_iter().enumerate() {
        let mut point = INITIAL_POINT;
        let mut physical_time = 0.0;

        for _ in 0..TOTAL_STEPS {
            let [x, y] = point;
            let radius_squared = x * x + y * y;
            let dx = mu * x - ANGULAR_FREQUENCY * y - radius_squared * x;
            let dy = ANGULAR_FREQUENCY * x + mu * y - radius_squared * y;
            point = [x + TIME_STEP * dx, y + TIME_STEP * dy];
            physical_time += TIME_STEP;
        }

        println!(
            "[naive] task={task} mu={mu} final_step={TOTAL_STEPS} final_time={physical_time} final_point=[{}, {}] final_radius={}",
            point[0],
            point[1],
            point[0].hypot(point[1])
        );
    }
}
