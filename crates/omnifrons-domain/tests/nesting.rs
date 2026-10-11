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

/// `relate` still accepts the scan root `"."` and an ordinary nested
/// path like `"a/b"` -- the degenerate-shape checks below reject only
/// the new malformed shapes, never these.
#[test]
fn relate_still_accepts_the_scan_root_and_an_ordinary_nested_path() {
    assert!(nesting::relate(&["."], &[]).is_ok());
    assert!(nesting::relate(&["a/b"], &[]).is_ok());
}

/// `relate` refuses an empty root: there is nothing to resolve against
/// the scan root.
#[test]
fn relate_rejects_an_empty_root() {
    assert_eq!(nesting::relate(&[""], &[]), Err(NestingError::EmptyPath));
}

/// `relate` refuses a root prefixed with a redundant `./`: the scan
/// root is spelled exactly `.`, never as a prefix of another path.
#[test]
fn relate_rejects_a_dot_slash_prefixed_root() {
    assert_eq!(
        nesting::relate(&["./a"], &[]),
        Err(NestingError::DotSlashPrefix("./a".to_string()))
    );
}

/// `relate` refuses a root with a trailing slash.
#[test]
fn relate_rejects_a_trailing_slash_root() {
    assert_eq!(
        nesting::relate(&["a/"], &[]),
        Err(NestingError::TrailingSlash("a/".to_string()))
    );
}

/// `relate` refuses a root with a doubled slash.
#[test]
fn relate_rejects_a_doubled_slash_root() {
    assert_eq!(
        nesting::relate(&["a//b"], &[]),
        Err(NestingError::DoubledSlash("a//b".to_string()))
    );
}

/// `relate` refuses a root with a bare `.` component in the middle --
/// only the whole path may be `.`, naming the scan root itself.
#[test]
fn relate_rejects_a_dot_component_in_the_middle() {
    assert_eq!(
        nesting::relate(&["a/./b"], &[]),
        Err(NestingError::DotComponent("a/./b".to_string()))
    );
}

/// `relate` takes the nearest (first) of two outside ancestors as the
/// parent of every root that has no inner parent -- never the second,
/// even though both `roots` entries fall back to it the same way.
#[test]
fn relate_uses_the_nearest_of_two_outside_ancestors_for_every_parentless_root() {
    let roots = ["a", "b"];
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
    let relations = nesting::relate(&roots, &outer).unwrap();

    for relation in &relations {
        assert_eq!(relation.parent, Some(Parent::Outside("near".to_string())));
    }
}

/// A port scanning a sub-directory discovers only a non-`"."` root
/// (`roots = ["a"]`); a non-empty `outer` still makes `a`'s parent the
/// outside ancestor, with consent required -- the same fallback `"."`
/// alone gets.
#[test]
fn relate_gives_a_non_dot_only_root_its_outside_ancestor_as_parent() {
    let roots = ["a"];
    let outer = [Ancestor {
        path: "atlas".to_string(),
        identity_files: vec!["AGENTS.md".to_string(), "CLAUDE.md".to_string()],
    }];
    let relations = nesting::relate(&roots, &outer).unwrap();

    let a = relations.iter().find(|r| r.root == "a").unwrap();
    assert_eq!(a.parent, Some(Parent::Outside("atlas".to_string())));
    assert!(a.consent_required());
}

/// The same sub-directory-only scan with no outside ancestors leaves
/// `a` without a parent at all.
#[test]
fn relate_gives_a_non_dot_only_root_no_parent_without_outer_ancestors() {
    let roots = ["a"];
    let relations = nesting::relate(&roots, &[]).unwrap();

    let a = relations.iter().find(|r| r.root == "a").unwrap();
    assert_eq!(a.parent, None);
    assert!(!a.consent_required());
}

/// `nearest_root` is non-fallible and trusts its scan-root-relative
/// precondition: given an absolute `path`, `"."`'s own empty
/// component list still matches as a prefix, so a `roots` slice
/// holding `"."` reports it as the containing root rather than
/// rejecting the input. Pins the documented precondition; this is not
/// a validity check.
#[test]
fn nearest_root_reports_dot_for_an_absolute_path_when_dot_is_a_root() {
    let roots = ["."];
    assert_eq!(nesting::nearest_root(&roots, "/etc/passwd"), Some("."));
}

/// The same precondition means a `roots` slice without `"."` has no
/// root to match an absolute `path` against, even though `"."` itself
/// would have matched it.
#[test]
fn nearest_root_returns_none_for_an_absolute_path_without_dot_among_roots() {
    let roots = ["a", "b"];
    assert_eq!(nesting::nearest_root(&roots, "/etc/passwd"), None);
}

/// `ancestor_chain` inherits the same precondition through
/// `nearest_root`: an absolute `root` argument still resolves its
/// inside-scan prefix via `"."` rather than being rejected.
#[test]
fn ancestor_chain_inherits_the_absolute_path_precondition() {
    let roots = [".", "child"];
    assert_eq!(nesting::ancestor_chain(&roots, "/etc/passwd"), ["."]);
}

/// `warning_chain` inherits the same precondition transitively: an
/// absolute `root` still resolves through `"."` before the outside
/// ancestors are appended.
#[test]
fn warning_chain_inherits_the_absolute_path_precondition() {
    let roots = ["."];
    let outer = [Ancestor {
        path: "atlas".to_string(),
        identity_files: vec!["AGENTS.md".to_string()],
    }];
    let chain = nesting::warning_chain(&roots, "/etc/passwd", &outer);
    assert_eq!(
        chain,
        [
            ChainLink::Root(".".to_string()),
            ChainLink::Outside(outer[0].clone()),
        ]
    );
}

/// `relate` lists a parent's `nested` children in the exact order
/// `roots` was given, not sorted -- a parent with three children given
/// out of alphabetical order still reports them back in that same,
/// non-alphabetical order.
#[test]
fn relate_nested_preserves_the_given_order_for_three_children() {
    let roots = [".", "charlie", "alpha", "bravo"];
    let relations = nesting::relate(&roots, &[]).unwrap();

    let root = relations.iter().find(|r| r.root == ".").unwrap();
    assert_eq!(root.nested, ["charlie", "alpha", "bravo"]);
}

/// Listing a grandchild, then its own child, then the scan root --
/// children before parents, the reverse of every other test's order --
/// still resolves every parent correctly and leaves every `nested`
/// list the same as the natural, parents-first order gives: `relate`'s
/// two-pass construction does not depend on a child appearing after
/// its parent in `roots`.
#[test]
fn relate_resolves_parents_correctly_when_children_precede_parents() {
    let roots = ["child/grandchild", "child", "."];
    let relations = nesting::relate(&roots, &[]).unwrap();

    let root = relations.iter().find(|r| r.root == ".").unwrap();
    assert_eq!(root.parent, None);
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
}

/// `relate` refuses a root that appears twice in `roots`, naming that
/// root.
#[test]
fn relate_rejects_a_duplicated_root() {
    assert_eq!(
        nesting::relate(&["a", "a"], &[]),
        Err(NestingError::DuplicateRoot("a".to_string()))
    );
}

/// A duplicated scan root `"."` is refused the same way as any other
/// duplicated root.
#[test]
fn relate_rejects_a_duplicated_scan_root() {
    assert_eq!(
        nesting::relate(&[".", "."], &[]),
        Err(NestingError::DuplicateRoot(".".to_string()))
    );
}

/// Distinct roots, even ones sharing a common prefix, are never
/// flagged as duplicates.
#[test]
fn relate_does_not_flag_distinct_roots_as_duplicates() {
    let roots = [".", "a", "a/b"];
    let relations = nesting::relate(&roots, &[]).unwrap();
    assert_eq!(relations.len(), roots.len());
}
