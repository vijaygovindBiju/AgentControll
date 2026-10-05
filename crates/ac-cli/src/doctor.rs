use ac_core::doctor::{Doctor, DoctorOpts, OverallStatus};
use anyhow::Result;

/// Runs AgentControll Doctor diagnostic suite and prints the result.
/// Returns process exit code (0 for Healthy/Warnings, 1 for Unhealthy).
pub async fn run_doctor(opts: DoctorOpts) -> Result<i32> {
    let doctor = Doctor::new(opts);
    let (report, output) = doctor.execute().await;
    print!("{output}");

    if report.status == OverallStatus::Unhealthy {
        Ok(1)
    } else {
        Ok(0)
    }
}
