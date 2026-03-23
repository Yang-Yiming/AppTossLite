use std::path::PathBuf;

use super::error::{Result, TossError};

#[derive(Debug, Clone)]
pub enum WorkflowEvent {
    Warning {
        message: String,
    },
    Building {
        project: String,
        scheme: String,
        device_udid: String,
    },
    BuildSucceeded,
    Installing {
        app_path: PathBuf,
        device_name: String,
    },
    Launching {
        bundle_id: String,
        device_name: String,
    },
    Signing {
        ipa_name: String,
        device_name: String,
    },
    ExtractedBundle {
        bundle_id: String,
        app_name: String,
    },
    UsingIdentity {
        identity_name: String,
    },
    SigningPlanStep {
        kind: String,
        original_bundle_id: String,
        final_bundle_id: String,
        profile_name: String,
    },
    TemporaryBundleId {
        original_bundle_id: String,
        temporary_bundle_id: String,
    },
    AutoProvisioning {
        kind: String,
        bundle_id: String,
        device_udid: String,
    },
    BundleIdRewritten {
        from: String,
        to: String,
    },
    CleanedTemporaryProfiles {
        count: usize,
    },
}

pub trait WorkflowAdapter {
    fn emit(&mut self, _event: WorkflowEvent) -> Result<()> {
        Ok(())
    }

    fn choose(&mut self, prompt: &str, items: &[String], default: usize) -> Result<Option<usize>>;
}

pub fn choose_index<A: WorkflowAdapter>(
    adapter: &mut A,
    prompt: &str,
    items: &[String],
    error: TossError,
) -> Result<usize> {
    match adapter.choose(prompt, items, 0)? {
        Some(index) => Ok(index),
        None => Err(error),
    }
}
