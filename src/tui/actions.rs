use dialoguer::{Input, Select};

use crate::core::actions;
use crate::core::config::Config;
use crate::core::error::{Result, TossError};

pub fn install(config: &Config) -> Result<()> {
    actions::install(config, None, None, None, false)
}

pub fn launch(config: &Config) -> Result<()> {
    actions::launch(config, None, None)
}

pub fn run(config: &Config) -> Result<()> {
    actions::run(config, None, None, None, false)
}

pub fn sign(config: &Config) -> Result<()> {
    let ipa_path: String = Input::new()
        .with_prompt("IPA file path")
        .interact_text()
        .map_err(|e| TossError::UserCancelled(e.to_string()))?;

    let launch_items = &["Install only", "Install + Launch"];
    let launch_sel = Select::new()
        .with_prompt("After signing")
        .items(launch_items)
        .default(0)
        .interact()
        .map_err(|e| TossError::UserCancelled(e.to_string()))?;

    actions::sign_ipa(
        config,
        std::path::Path::new(ipa_path.trim()),
        None,
        None,
        None,
        launch_sel == 1,
    )
}
