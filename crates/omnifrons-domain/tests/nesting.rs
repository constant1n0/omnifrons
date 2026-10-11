//! Nested agent-root relation contracts (ADR-0005 sub-slice 1d):
//! [`omnifrons_domain::nesting`] relates roots a port has already
//! discovered -- `relate`'s parent/nested wiring and consent, and the
//! ancestor and warning chains above a root.
//!
//! Translated from the relation parts of three of the prototype's
//! tests: `test_scan_nested_boundaries`,
//! `test_scan_nested_chain_on_nara_shape`, and
//! `test_scan_records_ancestors_outside_root`. Two sibling prototype
//! tests stay with the `DefinitionTree` port instead (sub-slices
//! 1e/1f), since they assert on the directory walk and the home
//! boundary, not on the relation this module computes:
//! `test_scan_prunes_ignored_and_host_config_dirs` and
//! `test_home_user_memory_is_not_an_ancestor`.

use omnifrons_domain::nesting::{self, Ancestor, ChainLink, NestingError, Parent};

/// The prototype's `test_scan_nested_boundaries`: scanning the outer
/// root together with one directly nested root wires the outer root as
/// the nested root's parent, lists the nested root under the outer
/// root's `nested`, and derives `consent_required` from `parent` alone
/// -- false for the outer root, true for the nested one.
#[test]
fn relate_wires_one_nested_pair_and_derives_consent() {
    let roots = [".", "echo"];
    let relations = nesting::relate(&roots, &[]).unwrap();

    let outer = relations.iter().find(|r| r.root == ".").unwrap();
    assert_eq!(outer.parent, None);
    assert_eq!(outer.nested, ["echo"]);
    assert!(!outer.consent_required());

    let nested = relations.iter().find(|r| r.root == "echo").unwrap();
    assert_eq!(nested.parent, Some(Parent::Root(".".to_string())));
    assert!(nested.nested.is_empty());
    assert!(nested.consent_required());
}

/// The prototype's `test_scan_nested_chain_on_nara_shape`: a
/// three-level chain, scan root / child / grandchild, wires the
/// grandchild's parent to the child (not the scan root), and
/// `ancestor_chain` reports both enclosing roots nearest first.
#[test]
fn relate_chains_three_levels_and_ancestor_chain_is_nearest_first() {
    let roots = [".", "child", "child/grandchild"];
    let relations = nesting::relate(&roots, &[]).unwrap();

    let root = relations.iter().find(|r| r.root == ".").unwrap();
    assert_eq!(root.nested, ["child"]);

    let child = relations.iter().find(|r| r.root == "child").unwrap();
    assert_eq!(child.parent, Some(Parent::Root(".".to_string())));
    assert_eq!(child.nested, ["child/grandchild"]);

    let grandchild = relations
        .iter()
        .find(|r| r.root == "child/grandchild")
        .unwrap();
    assert_eq!(grandchild.parent, Some(Parent::Root("child".to_string())));
    assert!(grandchild.nested.is_empty());

    assert_eq!(
        nesting::ancestor_chain(&roots, "child/grandchild"),
        ["child", "."]
    );
}

/// The prototype's `test_scan_records_ancestors_outside_root`: scanning
/// the nested root alone, with the outer root only as an ancestor
/// outside the scan, makes that outside ancestor the nested root's
/// parent -- with `consent_required` still true.
#[test]
fn relate_makes_the_nearest_outside_ancestor_the_parent() {
    let roots = ["."];
    let outer = [Ancestor {
        path: "atlas".to_string(),
        identity_files: vec!["AGENTS.md".to_string(), "CLAUDE.md".to_string()],
    }];
    let relations = nesting::relate(&roots, &outer).unwrap();

    let echo = relations.iter().find(|r| r.root == ".").unwrap();
    assert_eq!(echo.parent, Some(Parent::Outside("atlas".to_string())));
    assert!(echo.consent_required());
}

/// `nearest_root` picks the deepest of several roots that all contain
/// the same path.
#[test]
fn nearest_root_picks_the_deepest_containing_root() {
    let roots = [".", "a", "a/b"];
    assert_eq!(nesting::nearest_root(&roots, "a/b/c"), Some("a/b"));
}

/// `"."`, the scan root, contains every path -- even one with no other
/// owner.
#[test]
fn nearest_root_dot_is_the_root_of_everything() {
    let roots = ["."];
    assert_eq!(nesting::nearest_root(&roots, "deep/nested/path"), Some("."));
}

/// A path outside every given root has no owner.
#[test]
fn nearest_root_returns_none_outside_every_root() {
    let roots = ["a", "b"];
    assert_eq!(nesting::nearest_root(&roots, "c/d"), None);
}

/// `relate` refuses a root that is not scan-root-relative: absolute, or
/// carrying a `..` component.
#[test]
fn relate_rejects_absolute_and_parent_referencing_roots() {
    assert_eq!(
        nesting::relate(&["/etc"], &[]),
        Err(NestingError::AbsolutePath("/etc".to_string()))
    );
    assert_eq!(
        nesting::relate(&["a/../b"], &[]),
        Err(NestingError::ParentComponent("a/../b".to_string()))
    );
}

/// `warning_chain` for a nested root composes its inside-the-scan
/// ancestor chain first, then the outside ancestors in the order given
/// -- the nearer outside ancestor before the farther one.
#[test]
fn warning_chain_orders_inside_then_outside_ancestors() {
    let roots = [".", "child"];
    let outer = [
        Ancestor {
            path: "near".to_string(),
            identity_files: vec!["AGENTS.md".to_string()],
        },
        Ancestor {
            path: "far".to_string(),
            identity_files: vec!["CLAUDE.md".to_string()],
        },
    ];
    let chain = nesting::warning_chain(&roots, "child", &outer);
    assert_eq!(
        chain,
        [
            ChainLink::Root(".".to_string()),
            ChainLink::Outside(outer[0].clone()),
            ChainLink::Outside(outer[1].clone()),
        ]
    );
}
