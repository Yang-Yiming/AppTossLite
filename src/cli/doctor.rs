use crate::core::config::Config;
use crate::core::doctor;
use crate::core::error::{Result, TossError};

pub fn run(config: &Config) -> Result<()> {
    let report = doctor::collect(config)?;

    println!("toss doctor");
    println!();

    for section in &report.sections {
        println!("{}", section.title);
        for line in &section.lines {
            println!("  [{:<4}] {:<20} {}", line.status, line.label, line.detail);
        }
        println!();
    }

    println!(
        "Summary: {} failure(s), {} warning(s)",
        report.failures, report.warnings
    );

    if report.failures > 0 {
        Err(TossError::Config(format!(
            "doctor found {} failure(s) and {} warning(s)",
            report.failures, report.warnings
        )))
    } else {
        Ok(())
    }
}
