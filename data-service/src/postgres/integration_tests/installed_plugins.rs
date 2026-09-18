//! `InstalledPluginReader`/`InstalledPluginWriter`, against a real database.
//!
//! The boot loader's own tests run against an in-memory fake, which proves
//! the policy - what loads, what is refused, what is written down - and
//! nothing at all about the SQL underneath it. These are the other half: the
//! upsert really does re-enable on upgrade, the disable really does persist,
//! and the audit trail really does outlive the plugin it describes.

use uuid::Uuid;

use crate::installed_plugins::{
    InstalledPluginReader, InstalledPluginWriter, NewInstalledPlugin, NewPluginEvent,
    PluginEventKind,
};
use crate::postgres::PgDataService;

async fn service() -> Option<PgDataService> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .ok()?;
    Some(PgDataService::new(pool))
}

/// Unique per test run: these tables are server-wide, with no store or user
/// to scope them by, so two tests using the same literal id would collide on
/// the primary key against a shared database.
fn unique_id(suffix: &str) -> String {
    format!("cash.random.test-{}-{suffix}", Uuid::new_v4().simple())
}

fn new_plugin(id: &str, version: &str) -> NewInstalledPlugin {
    NewInstalledPlugin {
        id: id.to_string(),
        version: version.to_string(),
        manifest_toml: format!("id = \"{id}\"\nversion = \"{version}\"\nkind = \"action\"\n"),
        artifact_sha256: "a".repeat(64),
        db_role_password: None,
    }
}

#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn an_install_reads_back_exactly_what_was_written() {
    let Some(svc) = service().await else {
        return;
    };
    let id = unique_id("install");

    svc.upsert_installed_plugin(&new_plugin(&id, "0.1.0"))
        .await
        .expect("install");

    let row = svc
        .get_installed_plugin(&id)
        .await
        .expect("read")
        .expect("the plugin should be installed");

    assert_eq!(row.version, "0.1.0");
    assert_eq!(row.artifact_sha256, "a".repeat(64));
    assert!(row.enabled);
    assert!(row.disabled_reason.is_none());
    assert!(
        row.manifest_toml.contains(&id),
        "the manifest must come back byte-for-byte; boot re-parses it"
    );

    svc.remove_installed_plugin(&id).await.expect("cleanup");
}

/// A disable survives being written and read back, reason included. This is
/// the property that makes crash-disable recoverable rather than a crash
/// loop, and it is the one thing an in-memory fake cannot vouch for.
#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn a_disable_and_its_reason_persist() {
    let Some(svc) = service().await else {
        return;
    };
    let id = unique_id("disable");
    svc.upsert_installed_plugin(&new_plugin(&id, "0.1.0"))
        .await
        .expect("install");

    assert!(
        svc.set_plugin_enabled(&id, false, Some("trapped 3 times in a row"))
            .await
            .expect("disable"),
        "disabling an installed plugin should report that it updated a row"
    );

    let row = svc.get_installed_plugin(&id).await.unwrap().unwrap();
    assert!(!row.enabled);
    assert_eq!(
        row.disabled_reason.as_deref(),
        Some("trapped 3 times in a row")
    );

    svc.remove_installed_plugin(&id).await.expect("cleanup");
}

/// Re-enabling clears the reason. A plugin that is on must not still carry
/// an explanation for being off - an admin reading one would reasonably
/// conclude it is still off.
#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn enabling_clears_the_reason_it_was_disabled_for() {
    let Some(svc) = service().await else {
        return;
    };
    let id = unique_id("reenable");
    svc.upsert_installed_plugin(&new_plugin(&id, "0.1.0"))
        .await
        .expect("install");
    svc.set_plugin_enabled(&id, false, Some("digest mismatch"))
        .await
        .expect("disable");

    // Passing a reason alongside `enabled = true` must not store one.
    svc.set_plugin_enabled(&id, true, Some("ignored"))
        .await
        .expect("enable");

    let row = svc.get_installed_plugin(&id).await.unwrap().unwrap();
    assert!(row.enabled);
    assert!(
        row.disabled_reason.is_none(),
        "an enabled plugin must not carry a disabled reason, got {:?}",
        row.disabled_reason
    );

    svc.remove_installed_plugin(&id).await.expect("cleanup");
}

/// An upgrade re-enables a plugin a previous version's crash had disabled.
///
/// Without this, installing a fixed build would leave the plugin off with a
/// reason describing a version that is no longer on disk - and the one
/// action most likely to fix a broken plugin would appear to do nothing.
#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn an_upgrade_re_enables_a_plugin_the_previous_version_crashed() {
    let Some(svc) = service().await else {
        return;
    };
    let id = unique_id("upgrade");
    svc.upsert_installed_plugin(&new_plugin(&id, "0.1.0"))
        .await
        .expect("install");
    svc.set_plugin_enabled(&id, false, Some("0.1.0 trapped on every call"))
        .await
        .expect("disable");

    svc.upsert_installed_plugin(&new_plugin(&id, "0.2.0"))
        .await
        .expect("upgrade");

    let row = svc.get_installed_plugin(&id).await.unwrap().unwrap();
    assert_eq!(row.version, "0.2.0");
    assert!(
        row.enabled,
        "an upgrade must not inherit the old build's disable"
    );
    assert!(row.disabled_reason.is_none());

    svc.remove_installed_plugin(&id).await.expect("cleanup");
}

/// The audit trail outlives the plugin. `plugin_events` deliberately has no
/// foreign key to `installed_plugins`, because the most useful thing it can
/// tell an admin is that a plugin was uninstalled - which a cascade would
/// erase along with the row.
#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn events_survive_the_plugin_being_uninstalled() {
    let Some(svc) = service().await else {
        return;
    };
    let id = unique_id("audit");
    let actor = Uuid::new_v4();

    svc.upsert_installed_plugin(&new_plugin(&id, "0.1.0"))
        .await
        .expect("install");
    svc.record_plugin_event(&NewPluginEvent {
        plugin_id: id.clone(),
        kind: PluginEventKind::Installed,
        version: Some("0.1.0".to_string()),
        detail: None,
        actor_user_id: Some(actor),
    })
    .await
    .expect("record install");
    svc.record_plugin_event(&NewPluginEvent {
        plugin_id: id.clone(),
        kind: PluginEventKind::LoadFailed,
        version: Some("0.1.0".to_string()),
        detail: Some("artifact digest mismatch".to_string()),
        // The host did this one to itself.
        actor_user_id: None,
    })
    .await
    .expect("record failure");

    assert!(
        svc.remove_installed_plugin(&id).await.expect("uninstall"),
        "uninstalling an installed plugin should report that it removed a row"
    );

    let events = svc.plugin_events(&id, 10).await.expect("read events");
    assert_eq!(
        events.len(),
        2,
        "both events must outlive the plugin they describe"
    );
    // Newest first.
    assert_eq!(events[0].event, "load_failed");
    assert_eq!(
        events[0].detail.as_deref(),
        Some("artifact digest mismatch")
    );
    assert!(
        events[0].actor_user_id.is_none(),
        "a host-initiated disable has no actor; naming one would be a fiction \
         the audit trail cannot later correct"
    );
    assert_eq!(events[1].event, "installed");
    assert_eq!(events[1].actor_user_id, Some(actor));

    sqlx::query("DELETE FROM plugin_events WHERE plugin_id = $1")
        .bind(&id)
        .execute(svc.pool())
        .await
        .expect("cleanup");
}

/// Uninstalling something that was never installed says so, rather than
/// reporting a success that did nothing.
#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn uninstalling_an_absent_plugin_reports_that_nothing_was_removed() {
    let Some(svc) = service().await else {
        return;
    };
    assert!(
        !svc.remove_installed_plugin(&unique_id("absent"))
            .await
            .expect("delete")
    );
}

/// So does enabling or disabling one. The admin endpoint these exist for
/// takes a plugin id from a request, and reporting success for an id that
/// does not exist is how an admin concludes they have disabled something
/// they have not.
#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn toggling_an_absent_plugin_reports_that_nothing_changed() {
    let Some(svc) = service().await else {
        return;
    };
    let absent = unique_id("absent-toggle");
    assert!(
        !svc.set_plugin_enabled(&absent, false, Some("never installed"))
            .await
            .expect("disable")
    );
    assert!(
        !svc.set_plugin_enabled(&absent, true, None)
            .await
            .expect("enable")
    );
}

/// An upgrade that does not supply a credential must keep the one the plugin
/// already had.
///
/// The alternative is silent: a re-install through a path that skipped role
/// provisioning would null the column, and the plugin would come back with no
/// database access and no error anywhere - working yesterday, broken today,
/// nothing in the logs.
#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn an_upgrade_without_a_credential_keeps_the_existing_one() {
    let Some(service) = service().await else {
        return;
    };
    let id = unique_id("keeps-credential");

    let mut first = new_plugin(&id, "0.1.0");
    first.db_role_password = Some("secret-one".to_string());
    InstalledPluginWriter::upsert_installed_plugin(&service, &first)
        .await
        .unwrap();

    // An upgrade that says nothing about the credential.
    let second = new_plugin(&id, "0.2.0");
    InstalledPluginWriter::upsert_installed_plugin(&service, &second)
        .await
        .unwrap();

    let stored = InstalledPluginReader::get_installed_plugin(&service, &id)
        .await
        .unwrap()
        .expect("still installed");
    assert_eq!(stored.version, "0.2.0", "the upgrade did apply");
    assert_eq!(
        stored.db_role_password,
        Some("secret-one".to_string()),
        "the upgrade stripped the plugin's database credential"
    );
}

/// And a supplied credential replaces the old one, or rotating it would be
/// impossible.
#[tokio::test]
#[ignore = "requires DATABASE_URL"]
async fn an_upgrade_with_a_credential_replaces_it() {
    let Some(service) = service().await else {
        return;
    };
    let id = unique_id("rotates-credential");

    let mut first = new_plugin(&id, "0.1.0");
    first.db_role_password = Some("old".to_string());
    InstalledPluginWriter::upsert_installed_plugin(&service, &first)
        .await
        .unwrap();

    let mut second = new_plugin(&id, "0.2.0");
    second.db_role_password = Some("new".to_string());
    InstalledPluginWriter::upsert_installed_plugin(&service, &second)
        .await
        .unwrap();

    let stored = InstalledPluginReader::get_installed_plugin(&service, &id)
        .await
        .unwrap()
        .expect("still installed");
    assert_eq!(stored.db_role_password, Some("new".to_string()));
}
