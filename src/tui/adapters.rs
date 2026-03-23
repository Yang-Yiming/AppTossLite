use dialoguer::Select;

use crate::core::error::{Result, TossError};
use crate::core::interaction::{WorkflowAdapter, WorkflowEvent};

pub struct DialoguerAdapter;

impl WorkflowAdapter for DialoguerAdapter {
    fn emit(&mut self, event: WorkflowEvent) -> Result<()> {
        render_event(event);
        Ok(())
    }

    fn choose(&mut self, prompt: &str, items: &[String], default: usize) -> Result<Option<usize>> {
        let selection = Select::new()
            .with_prompt(prompt)
            .items(items)
            .default(default)
            .interact()
            .map_err(|e| TossError::UserCancelled(e.to_string()))?;
        Ok(Some(selection))
    }
}

pub fn render_event(event: WorkflowEvent) {
    match event {
        WorkflowEvent::Warning { message } => eprintln!("warning: {}", message),
        WorkflowEvent::Building {
            project,
            scheme,
            device_udid,
        } => {
            println!("Building {} ({}) for {}...", project, scheme, device_udid);
        }
        WorkflowEvent::BuildSucceeded => println!("BUILD SUCCEEDED"),
        WorkflowEvent::Installing {
            app_path,
            device_name,
        } => {
            println!("Installed {} on {}.", app_path.display(), device_name);
        }
        WorkflowEvent::Launching {
            bundle_id,
            device_name,
        } => {
            println!("Launched {} on {}.", bundle_id, device_name);
        }
        WorkflowEvent::Signing {
            ipa_name,
            device_name,
        } => {
            println!("Signing {} -> {}...", ipa_name, device_name);
        }
        WorkflowEvent::ExtractedBundle {
            bundle_id,
            app_name,
        } => println!("Extracted {} ({}).", bundle_id, app_name),
        WorkflowEvent::UsingIdentity { identity_name } => println!("Identity: {}", identity_name),
        WorkflowEvent::SigningPlanStep {
            kind,
            original_bundle_id,
            final_bundle_id,
            profile_name,
        } => println!(
            "Plan: {} {} -> {} using {}",
            kind, original_bundle_id, final_bundle_id, profile_name
        ),
        WorkflowEvent::TemporaryBundleId {
            original_bundle_id,
            temporary_bundle_id,
        } => println!(
            "No usable profile for '{}' found, switching to temporary bundle ID '{}'.",
            original_bundle_id, temporary_bundle_id
        ),
        WorkflowEvent::AutoProvisioning {
            kind,
            bundle_id,
            device_udid,
        } => println!(
            "Auto-provisioning {} '{}' for device {}...",
            kind, bundle_id, device_udid
        ),
        WorkflowEvent::BundleIdRewritten { from, to } => println!("Bundle ID: {} -> {}", from, to),
        WorkflowEvent::CleanedTemporaryProfiles { count } => {
            println!("Cleaned {} temporary provisioning profile(s).", count)
        }
    }
}
