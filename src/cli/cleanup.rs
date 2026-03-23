use crate::core::config::Config;
use crate::core::error::Result;

pub fn run(config: &Config) -> Result<()> {
    crate::cli::clean::run_legacy_cleanup(config)
}
