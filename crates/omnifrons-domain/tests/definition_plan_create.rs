//! Domain tests for the create operation, the creation states and the
//! create refusals of the definition plan (ADR-0005 "Agent identity and
//! portable definition" decisions 3 and 6, sub-slice 3b). They port, as
//! behaviour, the prototype's creation-state, refusal, host-config-folder
//! and plan-id tests; every fixture here is neutral and generic.

use std::collections::BTreeMap;

use omnifrons_domain::definition::{FileShape, IdentityFile, IdentityKind, WRAPPER};
use omnifrons_domain::definition_create::{
    ANSWERS_FORM_TAG, AnswerStatus, Answers, CREATE_TEMPLATE_VERSION, CreateContext, CreationState,
    Field, RawValue, answers_canonical, creation_state, render,
};
use omnifrons_domain::definition_plan::{
    CANONICAL_FORM_TAG, InputFingerprint, Operation, PlanRefusal, SourceProblem, canonical_bytes,
    home_target_message, host_config_folder, host_config_index, host_config_target_message,
    identity_exists_message, inside_host_config_message, nested_create_message,
    symlink_target_message,
};
use omnifrons_domain::executable::Sha256Digest;

// ---------------------------------------------------------------- fixtures

fn text(value: &str) -> RawValue {
    RawValue::Text(value.to_string())
}

fn item(body: &str, said: &str, status: &str) -> RawValue {
    RawValue::Object(vec![
        ("text".to_string(), text(body)),
        ("said".to_string(), text(said)),
        ("status".to_string(), text(status)),
    ])
}

/// The five required fields, with the given name and role text.
fn pairs(name: &str, role: &str, said: &str) -> Vec<(String, RawValue)> {
    vec![
        ("name".to_string(), item(name, said, "confirmed")),
        ("role".to_string(), item(role, said, "confirmed")),
        (
            "deliverables".to_string(),
            RawValue::List(vec![item("Propose visit slots.", said, "confirmed")]),
        ),
        (
            "authority".to_string(),
            RawValue::List(vec![item(
                "Draft and advise only; never publish, send or spend money.",
                said,
                "confirmed",
            )]),
        ),
        (
            "work_language".to_string(),
            item("the language of each user", said, "confirmed"),
        ),
    ]
}

fn answers_from(pairs: Vec<(String, RawValue)>) -> Answers {
    Answers::validate(&RawValue::Object(pairs)).expect("fixture answers are valid")
}

fn base_answers() -> Answers {
    answers_from(pairs("Harbor", "a scheduling assistant", "said"))
}

fn context(children: &[&str], enclosed: bool) -> CreateContext {
    CreateContext::new(
        children.iter().map(|c| (*c).to_string()).collect(),
        enclosed,
    )
    .expect("fixture context is valid")
}

/// A stand-in for the application's hasher: any injective-enough spread
/// of the bytes works for comparing plan texts.
fn fake_digest(bytes: &[u8]) -> Sha256Digest {
    let mut out = [0u8; 32];
    let mut acc: u64 = 0xcbf2_9ce4_8422_2325;
    for (index, byte) in bytes.iter().enumerate() {
        acc = (acc ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
        out[index % 32] ^= acc.to_be_bytes()[3];
    }
    Sha256Digest(out)
}

fn regular(kind: IdentityKind, bytes: &[u8]) -> IdentityFile {
    IdentityFile {
        kind,
        bytes: bytes.to_vec(),
        shape: FileShape::Regular,
    }
}

fn generated(answers: &Answers, ctx: &CreateContext) -> Operation {
    let rendered = render(answers, ctx);
    Operation::WriteGenerated {
        template_version: CREATE_TEMPLATE_VERSION.to_string(),
        answers_digest: fake_digest(&answers_canonical(answers)),
        after: fake_digest(rendered.as_bytes()),
        before: InputFingerprint::Absent,
    }
}

fn plan_text(operations: &[Operation], inputs: &BTreeMap<String, InputFingerprint>) -> Vec<u8> {
    canonical_bytes(b"/w", "agent", inputs, operations)
}

// ------------------------------------------------------- canonical line

#[test]
fn the_canonical_form_tag_is_v4() {
    assert_eq!(CANONICAL_FORM_TAG, "omnifrons-plan-v4");
}

#[test]
fn a_generated_write_has_its_own_canonical_line() {
    let operation = Operation::WriteGenerated {
        template_version: "adr-0005-create-v1".to_string(),
        answers_digest: Sha256Digest([0x11; 32]),
        after: Sha256Digest([0x22; 32]),
        before: InputFingerprint::Absent,
    };
    assert_eq!(
        operation.canonical(),
        format!(
            "write-generated path=AGENTS.md template=adr-0005-create-v1 answers={} after={} \
             before=absent",
            "11".repeat(32),
            "22".repeat(32)
        )
    );
}

#[test]
fn a_generated_write_reports_its_path_after_and_before() {
    let operation = generated(&base_answers(), &context(&[], false));
    assert_eq!(operation.path(), "AGENTS.md");
    assert_eq!(operation.before(), &InputFingerprint::Absent);
    let Operation::WriteGenerated { after, .. } = &operation else {
        panic!("not a generated write");
    };
    assert_eq!(operation.after(), after);
}

#[test]
fn the_plan_text_starts_with_the_new_tag_and_lists_the_generated_line() {
    let operation = generated(&base_answers(), &context(&[], false));
    let text = String::from_utf8(plan_text(
        std::slice::from_ref(&operation),
        &BTreeMap::new(),
    ))
    .unwrap();
    assert!(text.starts_with("omnifrons-plan-v4\n"), "{text}");
    assert!(
        text.contains("operations 1\nwrite-generated path=AGENTS.md "),
        "{text}"
    );
}

// ------------------------------------------------------ answers digest

#[test]
fn the_answers_form_starts_with_its_tag_and_lists_every_field_in_order() {
    let form = String::from_utf8(answers_canonical(&base_answers())).unwrap();
    assert!(form.starts_with(&format!("{ANSWERS_FORM_TAG}\n")), "{form}");
    let keys: Vec<&str> = form
        .lines()
        .filter_map(|line| line.strip_prefix("field "))
        .map(|rest| rest.split(' ').next().unwrap())
        .collect();
    let expected: Vec<&str> = Field::ALL.iter().map(|field| field.key()).collect();
    assert_eq!(keys, expected);
    assert!(form.contains("item 6:Harbor confirmed\n"), "{form}");
}

#[test]
fn the_answers_form_is_independent_of_the_order_of_the_fields() {
    let forward = answers_from(pairs("Harbor", "a scheduling assistant", "said"));
    let mut reversed_pairs = pairs("Harbor", "a scheduling assistant", "said");
    reversed_pairs.reverse();
    let reversed = answers_from(reversed_pairs);
    assert_eq!(answers_canonical(&forward), answers_canonical(&reversed));
}

#[test]
fn the_answers_form_ignores_the_users_words() {
    let first = answers_from(pairs("Harbor", "a scheduling assistant", "first words"));
    let second = answers_from(pairs("Harbor", "a scheduling assistant", "other words"));
    assert_eq!(answers_canonical(&first), answers_canonical(&second));
}

#[test]
fn the_answers_form_changes_with_every_answer_text() {
    let base = answers_canonical(&base_answers());
    let renamed = answers_from(pairs("Dock", "a scheduling assistant", "said"));
    let reroled = answers_from(pairs("Harbor", "a booking assistant", "said"));
    assert_ne!(base, answers_canonical(&renamed));
    assert_ne!(base, answers_canonical(&reroled));
    assert_ne!(answers_canonical(&renamed), answers_canonical(&reroled));
}

#[test]
fn the_answers_form_changes_with_a_status() {
    let mut assumed = pairs("Harbor", "a scheduling assistant", "said");
    assumed.push((
        "tone".to_string(),
        item("Warm and brief.", "said", "confirmed"),
    ));
    let mut assumed_tone = assumed.clone();
    assumed_tone.pop();
    assumed_tone.push((
        "tone".to_string(),
        item("Warm and brief.", "said", "assumption"),
    ));
    let confirmed = answers_from(assumed);
    let unconfirmed = answers_from(assumed_tone);
    assert_eq!(
        unconfirmed.items(Field::Tone)[0].status,
        AnswerStatus::Assumption
    );
    assert_ne!(
        answers_canonical(&confirmed),
        answers_canonical(&unconfirmed)
    );
}

#[test]
fn an_unanswered_optional_field_differs_from_an_answered_one() {
    let mut with_tone = pairs("Harbor", "a scheduling assistant", "said");
    with_tone.push(("tone".to_string(), item("Warm.", "said", "confirmed")));
    assert_ne!(
        answers_canonical(&base_answers()),
        answers_canonical(&answers_from(with_tone))
    );
}

// ------------------------------------------------------ creation states

fn rendered_bytes() -> Vec<u8> {
    render(&base_answers(), &context(&[], false)).into_bytes()
}

#[test]
fn a_folder_without_identity_files_is_new() {
    assert_eq!(creation_state(&[], &rendered_bytes()), CreationState::New);
}

#[test]
fn the_rendered_agents_file_alone_is_a_missing_wrapper() {
    let rendered = rendered_bytes();
    let files = [regular(IdentityKind::Agents, &rendered)];
    assert_eq!(
        creation_state(&files, &rendered),
        CreationState::WrapperMissing
    );
}

#[test]
fn the_rendered_agents_file_with_the_wrapper_is_already_created() {
    let rendered = rendered_bytes();
    let files = [
        regular(IdentityKind::Agents, &rendered),
        regular(IdentityKind::Claude, WRAPPER),
    ];
    assert_eq!(
        creation_state(&files, &rendered),
        CreationState::AlreadyCreated
    );
}

#[test]
fn an_agents_file_that_is_not_the_rendered_one_is_an_existing_identity() {
    let files = [regular(IdentityKind::Agents, b"# Something else\n")];
    assert_eq!(
        creation_state(&files, &rendered_bytes()),
        CreationState::IdentityExists
    );
}

#[test]
fn a_claude_file_that_is_not_the_exact_wrapper_is_an_existing_identity() {
    let rendered = rendered_bytes();
    for claude in [&b"@AGENTS.md"[..], b"@AGENTS.md\n\n", b"# Notes\n"] {
        let files = [
            regular(IdentityKind::Agents, &rendered),
            regular(IdentityKind::Claude, claude),
        ];
        assert_eq!(
            creation_state(&files, &rendered),
            CreationState::IdentityExists,
            "{claude:?}"
        );
    }
}

#[test]
fn a_lone_claude_file_is_an_existing_identity() {
    let files = [regular(IdentityKind::Claude, WRAPPER)];
    assert_eq!(
        creation_state(&files, &rendered_bytes()),
        CreationState::IdentityExists
    );
}

#[test]
fn every_identity_kind_alone_is_an_existing_identity() {
    let rendered = rendered_bytes();
    for kind in IdentityKind::ALL {
        let files = [regular(kind, b"# Existing\n")];
        assert_eq!(
            creation_state(&files, &rendered),
            CreationState::IdentityExists,
            "{kind}"
        );
    }
}

#[test]
fn any_other_kind_beside_the_writers_own_result_is_an_existing_identity() {
    let rendered = rendered_bytes();
    for kind in [
        IdentityKind::Gemini,
        IdentityKind::Qwen,
        IdentityKind::ClaudeDir,
        IdentityKind::AgentsOverride,
    ] {
        for with_wrapper in [false, true] {
            let mut files = vec![regular(IdentityKind::Agents, &rendered)];
            if with_wrapper {
                files.push(regular(IdentityKind::Claude, WRAPPER));
            }
            files.push(regular(kind, WRAPPER));
            assert_eq!(
                creation_state(&files, &rendered),
                CreationState::IdentityExists,
                "{kind} wrapper={with_wrapper}"
            );
        }
    }
}

#[test]
fn a_symlinked_or_irregular_identity_file_is_an_existing_identity() {
    let rendered = rendered_bytes();
    for shape in [
        FileShape::Symlink {
            target: "elsewhere.md".to_string(),
        },
        FileShape::Irregular,
    ] {
        let agents = IdentityFile {
            kind: IdentityKind::Agents,
            bytes: Vec::new(),
            shape: shape.clone(),
        };
        assert_eq!(
            creation_state(&[agents], &rendered),
            CreationState::IdentityExists
        );
        let claude = IdentityFile {
            kind: IdentityKind::Claude,
            bytes: Vec::new(),
            shape,
        };
        let files = [regular(IdentityKind::Agents, &rendered), claude];
        assert_eq!(
            creation_state(&files, &rendered),
            CreationState::IdentityExists
        );
    }
}

#[test]
fn a_changed_answer_turns_the_previous_result_into_an_existing_identity() {
    let old = rendered_bytes();
    let new = render(
        &answers_from(pairs("Dock", "a scheduling assistant", "said")),
        &context(&[], false),
    )
    .into_bytes();
    let files = [
        regular(IdentityKind::Agents, &old),
        regular(IdentityKind::Claude, WRAPPER),
    ];
    assert_eq!(creation_state(&files, &new), CreationState::IdentityExists);
}

#[test]
fn every_creation_state_has_a_stable_token() {
    let tokens: Vec<&str> = [
        CreationState::New,
        CreationState::WrapperMissing,
        CreationState::AlreadyCreated,
        CreationState::IdentityExists,
    ]
    .iter()
    .map(|state| state.as_str())
    .collect();
    assert_eq!(
        tokens,
        [
            "new",
            "wrapper_missing",
            "already_created",
            "identity_exists"
        ]
    );
}

// ------------------------------------------------- host config folders

#[test]
fn a_host_config_folder_is_recognised_at_any_depth() {
    assert!(host_config_folder(&["home", "me", ".claude"]));
    assert!(host_config_folder(&["home", ".claude", "rules", "team"]));
    assert!(host_config_folder(&[".kimi"]));
    assert!(host_config_folder(&["w", ".agents", "x"]));
    assert!(host_config_folder(&["w", ".cursor", "rules"]));
}

#[test]
fn the_host_config_match_ignores_case() {
    assert!(host_config_folder(&["home", ".Claude"]));
    assert!(host_config_folder(&["home", ".AGENTS", "x"]));
    assert!(host_config_folder(&["home", ".Cursor"]));
}

#[test]
fn ordinary_folders_are_not_host_config_folders() {
    assert!(!host_config_folder(&[]));
    assert!(!host_config_folder(&["home", "me", "work", "agent"]));
    assert!(!host_config_folder(&["home", "claude"]));
    assert!(!host_config_folder(&["home", ".github", "agent"]));
    assert!(!host_config_folder(&["home", "x.claude", ".claudes"]));
}

#[test]
fn the_deepest_host_config_component_is_the_one_reported() {
    assert_eq!(
        host_config_index(&["a", ".claude", "b", ".agents", "c"]),
        Some(3)
    );
    assert_eq!(host_config_index(&["a", ".claude"]), Some(1));
    assert_eq!(host_config_index(&["a", "b"]), None);
}

// ------------------------------------------------------------ refusals

#[test]
fn the_create_refusals_have_the_prototype_tokens() {
    assert_eq!(
        PlanRefusal::UnsafeTarget(String::new()).code(),
        "unsafe_target"
    );
    assert_eq!(
        PlanRefusal::IdentityExists(String::new()).code(),
        "identity_exists"
    );
    assert_eq!(
        PlanRefusal::InvalidSource(String::new()).code(),
        "invalid_source"
    );
    assert_eq!(
        PlanRefusal::NestedAgent(String::new()).code(),
        "nested_agent"
    );
}

#[test]
fn a_create_refusal_displays_its_token_and_message() {
    let refusal = PlanRefusal::UnsafeTarget("/h is the home directory".to_string());
    assert_eq!(refusal.message(), "/h is the home directory");
    assert_eq!(
        refusal.to_string(),
        "refused (unsafe_target): /h is the home directory"
    );
}

#[test]
fn the_unsafe_target_messages_name_which_target_it_is() {
    let symlink = symlink_target_message("/w/a", "/elsewhere");
    assert!(
        symlink.contains("/w/a is a symlink to /elsewhere"),
        "{symlink}"
    );
    let config = host_config_target_message("/w/.claude", ".claude");
    assert!(
        config.contains("host configuration folder (.claude/)"),
        "{config}"
    );
    let inside = inside_host_config_message("/w/.claude/rules/a", "/w/.claude", ".claude", "/w");
    assert!(
        inside.contains("inside the host configuration folder /w/.claude"),
        "{inside}"
    );
    assert!(inside.contains("configuration of /w"), "{inside}");
    let home = home_target_message("/h");
    assert!(home.contains("/h is the home directory"), "{home}");
}

#[test]
fn the_identity_exists_message_names_the_files() {
    let message = identity_exists_message(
        "/w/a",
        &["AGENTS.md", "GEMINI.md"],
        " whose AGENTS.md is not what these answers render",
    );
    assert!(message.contains("(AGENTS.md, GEMINI.md)"), "{message}");
    assert!(
        message.contains("existing agent whose AGENTS.md is not"),
        "{message}"
    );
}

#[test]
fn the_nested_message_names_the_enclosing_agent() {
    let message = nested_create_message("/w/a/b", "/w/a");
    assert!(
        message.contains("/w/a/b is inside the agent at /w/a"),
        "{message}"
    );
}

#[test]
fn each_source_problem_becomes_an_invalid_source_naming_the_entry() {
    let missing = SourceProblem::Missing.refusal(1, "docs/brief.md", "/w/a");
    assert_eq!(missing.code(), "invalid_source");
    assert_eq!(
        missing.message(),
        "sources[1].text: docs/brief.md does not exist in /w/a"
    );
    let link = SourceProblem::Symlink {
        shown: "docs".to_string(),
    }
    .refusal(0, "docs/brief.md", "/w/a");
    assert!(
        link.message().contains("docs is a symlink"),
        "{}",
        link.message()
    );
    let inside = SourceProblem::InsideChild {
        child: "kid".to_string(),
    }
    .refusal(2, "kid/x.md", "/w/a");
    assert!(
        inside
            .message()
            .starts_with("sources[2].text: kid/x.md is inside the child agent kid/")
    );
    let around = SourceProblem::ContainsChild {
        child: "kid".to_string(),
    }
    .refusal(0, "docs", "/w/a");
    assert!(
        around
            .message()
            .contains("docs contains the child agent kid/")
    );
}

// --------------------------------------------------------------- plan id

fn parent_inputs(fingerprint: InputFingerprint) -> BTreeMap<String, InputFingerprint> {
    BTreeMap::from([("parent/AGENTS.md".to_string(), fingerprint)])
}

#[test]
fn the_plan_text_is_stable_for_the_same_answers() {
    let ctx = context(&[], false);
    let first = plan_text(&[generated(&base_answers(), &ctx)], &BTreeMap::new());
    let again = plan_text(&[generated(&base_answers(), &ctx)], &BTreeMap::new());
    assert_eq!(first, again);
}

#[test]
fn the_plan_text_is_independent_of_the_key_order_and_the_users_words() {
    let ctx = context(&[], false);
    let mut shuffled = pairs("Harbor", "a scheduling assistant", "other words");
    shuffled.reverse();
    let first = plan_text(&[generated(&base_answers(), &ctx)], &BTreeMap::new());
    let second = plan_text(
        &[generated(&answers_from(shuffled), &ctx)],
        &BTreeMap::new(),
    );
    assert_eq!(first, second);
}

#[test]
fn the_plan_text_changes_with_every_answer() {
    let ctx = context(&[], false);
    let base = plan_text(&[generated(&base_answers(), &ctx)], &BTreeMap::new());
    let renamed = answers_from(pairs("Dock", "a scheduling assistant", "said"));
    let reroled = answers_from(pairs("Harbor", "a booking assistant", "said"));
    let mut with_tone = pairs("Harbor", "a scheduling assistant", "said");
    with_tone.push(("tone".to_string(), item("Warm.", "said", "confirmed")));
    for changed in [renamed, reroled, answers_from(with_tone)] {
        assert_ne!(
            base,
            plan_text(&[generated(&changed, &ctx)], &BTreeMap::new())
        );
    }
}

#[test]
fn the_plan_text_changes_with_the_rendered_body_and_the_skills_appended_to_it() {
    let ctx = context(&[], false);
    let answers = base_answers();
    let plain = generated(&answers, &ctx);
    let Operation::WriteGenerated {
        template_version,
        answers_digest,
        before,
        ..
    } = plain.clone()
    else {
        panic!("not a generated write");
    };
    let mut with_skills = render(&answers, &ctx);
    with_skills.push_str("\n<!-- skills block -->\n");
    let skilled = Operation::WriteGenerated {
        template_version,
        answers_digest,
        after: fake_digest(with_skills.as_bytes()),
        before,
    };
    assert_ne!(
        plan_text(&[plain], &BTreeMap::new()),
        plan_text(&[skilled], &BTreeMap::new())
    );
}

#[test]
fn the_plan_text_changes_with_the_children() {
    let none = plan_text(
        &[generated(&base_answers(), &context(&[], false))],
        &BTreeMap::new(),
    );
    let one = plan_text(
        &[generated(&base_answers(), &context(&["kid"], false))],
        &BTreeMap::new(),
    );
    let two = plan_text(
        &[generated(
            &base_answers(),
            &context(&["kid", "other"], false),
        )],
        &BTreeMap::new(),
    );
    assert_ne!(none, one);
    assert_ne!(one, two);
}

#[test]
fn the_plan_text_changes_with_the_enclosing_agent_and_its_fingerprint() {
    let alone = generated(&base_answers(), &context(&[], false));
    let enclosed = generated(&base_answers(), &context(&[], true));
    assert_ne!(
        plan_text(std::slice::from_ref(&alone), &BTreeMap::new()),
        plan_text(&[enclosed], &BTreeMap::new())
    );
    let first = parent_inputs(InputFingerprint::Sha256(Sha256Digest([1; 32])));
    let second = parent_inputs(InputFingerprint::Sha256(Sha256Digest([2; 32])));
    assert_ne!(
        plan_text(std::slice::from_ref(&alone), &first),
        plan_text(std::slice::from_ref(&alone), &second)
    );
    assert_ne!(
        plan_text(std::slice::from_ref(&alone), &BTreeMap::new()),
        plan_text(std::slice::from_ref(&alone), &first)
    );
}

#[test]
fn the_plan_text_changes_with_the_template_version_and_the_before_fingerprint() {
    let answers = base_answers();
    let ctx = context(&[], false);
    let base = generated(&answers, &ctx);
    let Operation::WriteGenerated {
        answers_digest,
        after,
        ..
    } = base.clone()
    else {
        panic!("not a generated write");
    };
    let other_template = Operation::WriteGenerated {
        template_version: "adr-0005-create-v2".to_string(),
        answers_digest,
        after,
        before: InputFingerprint::Absent,
    };
    let other_before = Operation::WriteGenerated {
        template_version: CREATE_TEMPLATE_VERSION.to_string(),
        answers_digest,
        after,
        before: InputFingerprint::Irregular,
    };
    let base_text = plan_text(&[base], &BTreeMap::new());
    assert_ne!(base_text, plan_text(&[other_template], &BTreeMap::new()));
    assert_ne!(base_text, plan_text(&[other_before], &BTreeMap::new()));
}
