use console::Style;

use crate::core::error::Result;
use crate::core::interaction::{WorkflowAdapter, WorkflowEvent};

pub struct StrictCliAdapter;

impl WorkflowAdapter for StrictCliAdapter {
    fn emit(&mut self, event: WorkflowEvent) -> Result<()> {
        render_event(event);
        Ok(())
    }

    fn choose(
        &mut self,
        _prompt: &str,
        _items: &[String],
        _default: usize,
    ) -> Result<Option<usize>> {
        Ok(None)
    }
}

pub fn render_event(event: WorkflowEvent) {
    match event {
        WorkflowEvent::Warning { message } => {
            let yellow = Style::new().yellow().bold();
            eprintln!("{} {}", yellow.apply_to("warning:"), message);
        }
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
            println!("Installing {} on {}...", app_path.display(), device_name);
        }
        WorkflowEvent::Launching {
            bundle_id,
            device_name,
        } => {
            println!("Launching {} on {}...", bundle_id, device_name);
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
        } => {
            println!("Extracted {} ({}).", bundle_id, app_name);
        }
        WorkflowEvent::UsingIdentity { identity_name } => {
            println!("Identity: {}", identity_name);
        }
        WorkflowEvent::SigningPlanStep {
            kind,
            original_bundle_id,
            final_bundle_id,
            profile_name,
        } => {
            println!(
                "Plan: {} {} -> {} using {}",
                kind, original_bundle_id, final_bundle_id, profile_name
            );
        }
        WorkflowEvent::TemporaryBundleId {
            original_bundle_id,
            temporary_bundle_id,
        } => {
            println!(
                "No usable profile for '{}' found, switching to temporary bundle ID '{}'.",
                original_bundle_id, temporary_bundle_id
            );
        }
        WorkflowEvent::AutoProvisioning {
            kind,
            bundle_id,
            device_udid,
        } => {
            println!(
                "Auto-provisioning {} '{}' for device {}...",
                kind, bundle_id, device_udid
            );
        }
        WorkflowEvent::BundleIdRewritten { from, to } => {
            println!("Bundle ID: {} -> {}", from, to);
        }
        WorkflowEvent::CleanedTemporaryProfiles { count } => {
            println!("Cleaned {} temporary provisioning profile(s).", count);
        }
    }
}
