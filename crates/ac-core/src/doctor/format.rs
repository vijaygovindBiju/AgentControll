use super::{CheckStatus, DoctorReport, OverallStatus};

pub fn render_json(report: &DoctorReport) -> String {
    serde_json::to_string_pretty(report).unwrap_or_else(|_| "{}".to_string())
}

pub fn render_human(report: &DoctorReport, deep: bool) -> String {
    let mut out = String::new();

    out.push_str(&format!(
        "AgentControll Doctor v{}{}\n",
        report.version,
        if deep { " (deep mode)" } else { "" }
    ));
    out.push_str(&format!("{}\n\n", "─".repeat(50)));

    for check in &report.checks {
        let symbol = match check.status {
            CheckStatus::Pass => "✓",
            CheckStatus::Warn => "⚠",
            CheckStatus::Fail => "✗",
            CheckStatus::Info => "ℹ",
        };

        out.push_str(&format!("{} {}\n", symbol, check.title));
        out.push_str(&format!("  {}\n", check.summary));

        if let Some(ref details) = check.details {
            for line in details.lines() {
                out.push_str(&format!("    {line}\n"));
            }
        }

        if let Some(ref remediation) = check.remediation {
            out.push_str(&format!("  Suggested action: {remediation}\n"));
        }

        out.push('\n');
    }

    out.push_str(&format!("{}\n\n", "─".repeat(50)));

    let status_str = match report.status {
        OverallStatus::Healthy => "HEALTHY",
        OverallStatus::Warnings => "WARNINGS",
        OverallStatus::Unhealthy => "UNHEALTHY",
    };

    out.push_str(&format!("Doctor result: {status_str}\n\n"));
    out.push_str(&format!("{} checks passed\n", report.pass_count()));
    out.push_str(&format!("{} warnings\n", report.warn_count()));
    out.push_str(&format!("{} errors\n", report.fail_count()));
    if report.info_count() > 0 {
        out.push_str(&format!("{} notices\n", report.info_count()));
    }

    out
}
