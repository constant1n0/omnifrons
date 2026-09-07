//! Runs the reusable `HarnessAdapter` contract over a fake test double.
//!
//! Only compiled with `--features contract-tests` (see
//! `omnifrons-app`'s `contract-tests` feature and
//! `docs/repository-layout.md` § Build and test commands).

#![cfg(feature = "contract-tests")]

use omnifrons_app::contract::harness_adapter::harness_adapter_contract;
use omnifrons_app::{
    AdapterDescriptor, EnvPlan, HarnessAdapter, LaunchPlan, LaunchPlanError, LaunchRequest,
};
use omnifrons_domain::adapter::{
    AdapterEvent, AdapterId, AgentPhase, PromptChannel, StdinPlan, TransportClass,
};
use omnifrons_domain::scope::ScopeMode;

/// A fake `HarnessAdapter`: recognizes exactly one literal line
/// (`"init"`), maps everything else to `Unknown`, matching the trait's own
/// "total" contract requirement.
struct FakeAdapter;

impl HarnessAdapter for FakeAdapter {
    fn describe(&self) -> AdapterDescriptor {
        AdapterDescriptor {
            id: AdapterId::stream_json_cli(),
            display_name: "Fake".to_string(),
            transport_class: TransportClass::StructuredStreamingCli,
            prompt_channel: PromptChannel::StdinThenClose,
            argv_template: vec![],
            declared_env: vec![],
            scope_mode: ScopeMode::Advisory,
            notes: String::new(),
        }
    }

    fn build_launch(&self, request: &LaunchRequest) -> Result<LaunchPlan, LaunchPlanError> {
        Ok(LaunchPlan {
            argv: vec![],
            env: EnvPlan::new(&[])?,
            cwd: request.workspace.clone(),
            stdin: StdinPlan::PipePromptThenClose,
            prompt: Some(request.prompt.clone()),
            scope_mode: ScopeMode::Advisory,
            transport: TransportClass::StructuredStreamingCli,
        })
    }

    fn parse_line(&self, line: &str) -> Vec<AdapterEvent> {
        if line == "init" {
            vec![AdapterEvent::State {
                phase: AgentPhase::Init,
                observations: vec![],
            }]
        } else {
            vec![AdapterEvent::Unknown {
                raw: line.as_bytes().to_vec(),
                truncated: false,
            }]
        }
    }
}

#[test]
fn fake_adapter_satisfies_harness_adapter_contract() {
    harness_adapter_contract(|| FakeAdapter);
}
