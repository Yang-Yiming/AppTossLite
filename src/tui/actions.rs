use dialoguer::{Input, Select};

use crate::core::actions;
use crate::core::config::Config;
use crate::core::error::{Result, TossError};
use crate::tui::adapters::DialoguerAdapter;

pub fn install(config: &Config) -> Result<()> {
    let mut adapter = DialoguerAdapter;
    actions::install(config, None, None, None, false, &mut adapter).map(|_| ())
}

pub fn launch(config: &Config) -> Result<()> {
    let mut adapter = DialoguerAdapter;
    actions::launch(config, None, None, &mut adapter).map(|_| ())
}

pub fn run(config: &Config) -> Result<()> {
    let mut adapter = DialoguerAdapter;
    actions::run(config, None, None, None, false, &mut adapter).map(|_| ())
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

    let mut adapter = DialoguerAdapter;
    actions::sign_ipa(
        config,
        std::path::Path::new(ipa_path.trim()),
        None,
        None,
        None,
        launch_sel == 1,
        &mut adapter,
    )
    .map(|_| ())
}
