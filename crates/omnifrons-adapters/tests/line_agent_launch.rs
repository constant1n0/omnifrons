//! `LineAgent::build_launch`: argv comes only from the descriptor's own
//! `argv_template`, the prompt never appears in argv (only via stdin), and
//! both built-in descriptors (`stream-json-cli`, `claude-code`) are
//! present in `catalog()`.

use omnifrons_adapters::LineAgent;
use omnifrons_adapters::line_agent::catalog;
use omnifrons_app::harness_adapter::HarnessAdapter as _;
use omnifrons_app::{LaunchRequest, WorkspaceRoot};
use omnifrons_domain::adapter::{AdapterId, AgentPrompt, PromptChannel, StdinPlan};

fn workspace() -> WorkspaceRoot {
    WorkspaceRoot::new(std::env::temp_dir()).expect("the system temp dir must be a valid workspace")
}

#[test]
fn both_built_in_descriptors_are_present() {
    let adapters = catalog();
    let ids: Vec<AdapterId> = adapters.iter().map(|a| a.describe().id).collect();
    assert!(ids.contains(&AdapterId::stream_json_cli()));
    assert!(ids.contains(&AdapterId::claude_code()));
    assert_eq!(
        ids.len(),
        2,
        "the built-in catalog must contain exactly two adapters"
    );
}

#[test]
fn build_launch_argv_is_exactly_the_descriptor_s_template() {
    for adapter in catalog() {
        let descriptor = adapter.describe();
        let prompt = AgentPrompt::new("do the thing").expect("valid prompt");
        let request = LaunchRequest {
            prompt: prompt.clone(),
            workspace: workspace(),
        };
        let plan = adapter
            .build_launch(&request)
            .expect("build_launch must succeed for a well-formed request");

        assert_eq!(
            plan.argv, descriptor.argv_template,
            "argv must be exactly the descriptor's own argv_template for {}",
            descriptor.id
        );
        for arg in &plan.argv {
            assert!(
                !arg.contains("do the thing"),
                "the prompt text must never appear in argv, found in argument {arg:?}"
            );
        }
    }
}

#[test]
fn build_launch_never_puts_the_prompt_in_argv_even_for_a_large_prompt() {
    let agent = LineAgent::new(
        catalog()
            .into_iter()
            .find(|a| a.describe().id == AdapterId::claude_code())
            .expect("claude-code must be present")
            .describe(),
    );
    let big_prompt_text = "secret-marker-".repeat(200);
    let prompt = AgentPrompt::new(big_prompt_text.clone()).expect("prompt within the cap");
    let request = LaunchRequest {
        prompt: prompt.clone(),
        workspace: workspace(),
    };

    let plan = agent
        .build_launch(&request)
        .expect("build_launch must succeed");

    assert!(
        plan.argv.iter().all(|arg| !arg.contains("secret-marker")),
        "the prompt must never leak into argv"
    );
    assert_eq!(
        plan.prompt,
        Some(prompt),
        "the prompt must be carried via LaunchPlan::prompt instead"
    );
    assert_eq!(plan.stdin, StdinPlan::PipePromptThenClose);
}

#[test]
fn both_built_ins_declare_stdin_then_close_as_their_prompt_channel() {
    for adapter in catalog() {
        assert_eq!(
            adapter.describe().prompt_channel,
            PromptChannel::StdinThenClose
        );
    }
}
