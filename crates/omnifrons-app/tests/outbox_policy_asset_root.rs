//! The policy's asset root identity (spike slice 5b): the project policy
//! names the asset root a publication is destined for (`asset_root_id`,
//! HAP-001 § Per project, as this slice binds it -- a spike default for
//! the asset binding the context scope will carry); the shipped default
//! policy declares none, so a fresh project cannot publish until its
//! policy names one (HAP-001-R6: never a fallback destination).

use omnifrons_app::outbox_policy::OutboxPolicy;
use omnifrons_domain::outbox::OutboxPath;
use omnifrons_domain::publication::AssetRootId;

#[test]
fn the_default_policy_declares_no_asset_root() {
    assert_eq!(OutboxPolicy::default_policy().asset_root_id(), None);
}

#[test]
fn a_policy_can_declare_its_asset_root() {
    let asset_root = AssetRootId::new("main").expect("valid");
    let policy = OutboxPolicy::new(OutboxPath::default_path(), Vec::new())
        .expect("valid")
        .with_asset_root(Some(asset_root.clone()));
    assert_eq!(policy.asset_root_id(), Some(&asset_root));
    assert_eq!(policy.outbox(), &OutboxPath::default_path());
}
