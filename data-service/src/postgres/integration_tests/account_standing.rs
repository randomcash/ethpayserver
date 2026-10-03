//! `AccountStandingStore`, against a real database.
//!
//! The property under test is that a lower version never replaces a higher
//! one. It lives in a single SQL statement, so only a real Postgres can show
//! it holds - including with two pushes racing.

use chrono::{Duration, Utc};
use uuid::Uuid;

use crate::account_standing::{
    AccountStanding, AccountStandingStore, ApplyOutcome, StandingDecision,
};
use crate::postgres::PgDataService;

async fn service() -> Option<PgDataService> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .connect(&database_url)
        .await
        .ok()?;
    Some(PgDataService::new(pool))
}

fn standing(account: Uuid, version: i64, good: bool) -> AccountStanding {
    AccountStanding {
        account_id: account,
        version,
        in_good_standing: good,
        paid_through: None,
        plan_name: format!("plan-v{version}"),
        checkout_url: Some("https://pay.example/checkout".into()),
    }
}

#[tokio::test]
#[ignore]
async fn a_lower_version_never_replaces_a_higher_one() {
    let Some(svc) = service().await else { return };
    let a = Uuid::new_v4();

    assert_eq!(
        svc.apply_account_standing(&standing(a, 6, true))
            .await
            .unwrap(),
        ApplyOutcome::Applied { version: 6 }
    );
    assert_eq!(
        svc.apply_account_standing(&standing(a, 5, false))
            .await
            .unwrap(),
        ApplyOutcome::Kept { held_version: 6 }
    );

    let held = svc.get_account_standing(a).await.unwrap().unwrap().standing;
    assert_eq!(held.version, 6);
    assert!(
        held.in_good_standing,
        "the stale push must not flip the standing"
    );
    assert_eq!(held.plan_name, "plan-v6");
}

#[tokio::test]
#[ignore]
async fn the_same_version_twice_changes_nothing_and_keeps_the_held_content() {
    let Some(svc) = service().await else { return };
    let a = Uuid::new_v4();

    svc.apply_account_standing(&standing(a, 6, true))
        .await
        .unwrap();
    let mut different = standing(a, 6, false);
    different.plan_name = "other".into();
    assert_eq!(
        svc.apply_account_standing(&different).await.unwrap(),
        ApplyOutcome::Kept { held_version: 6 }
    );

    let held = svc.get_account_standing(a).await.unwrap().unwrap().standing;
    assert!(held.in_good_standing);
    assert_eq!(held.plan_name, "plan-v6");
}

#[tokio::test]
#[ignore]
async fn a_higher_version_replaces_the_held_one() {
    let Some(svc) = service().await else { return };
    let a = Uuid::new_v4();

    svc.apply_account_standing(&standing(a, 1, true))
        .await
        .unwrap();
    svc.apply_account_standing(&standing(a, 2, false))
        .await
        .unwrap();

    let held = svc.get_account_standing(a).await.unwrap().unwrap().standing;
    assert_eq!((held.version, held.in_good_standing), (2, false));
}

#[tokio::test]
#[ignore]
async fn racing_pushes_leave_the_highest_version_whatever_the_order() {
    let Some(svc) = service().await else { return };
    for _ in 0..10 {
        let a = Uuid::new_v4();
        let svc = std::sync::Arc::new(svc.clone());
        let mut tasks = Vec::new();
        for v in 1..=12i64 {
            let svc = svc.clone();
            tasks.push(tokio::spawn(async move {
                svc.apply_account_standing(&standing(a, v, v % 2 == 0))
                    .await
                    .unwrap()
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
        let held = svc.get_account_standing(a).await.unwrap().unwrap().standing;
        assert_eq!(held.version, 12);
        assert!(
            held.in_good_standing,
            "content must be version 12's, not a loser's"
        );
    }
}

#[tokio::test]
#[ignore]
async fn only_a_repeat_of_the_held_version_counts_as_hearing_the_sender() {
    let Some(svc) = service().await else { return };
    let a = Uuid::new_v4();
    svc.apply_account_standing(&standing(a, 6, true))
        .await
        .unwrap();
    let backdate = |d: i64| {
        let pool = svc.pool().clone();
        async move {
            sqlx::query("UPDATE account_standing SET last_heard_at = now() - make_interval(days => $2) WHERE account_id = $1")
                .bind(a)
                .bind(d as i32)
                .execute(&pool)
                .await
                .unwrap();
        }
    };

    backdate(30).await;
    svc.apply_account_standing(&standing(a, 5, true))
        .await
        .unwrap();
    let silent = Utc::now()
        - svc
            .get_account_standing(a)
            .await
            .unwrap()
            .unwrap()
            .last_heard_at;
    assert!(
        silent > Duration::days(29),
        "a stale lower push is not a sign of life"
    );

    svc.apply_account_standing(&standing(a, 6, true))
        .await
        .unwrap();
    let silent = Utc::now()
        - svc
            .get_account_standing(a)
            .await
            .unwrap()
            .unwrap()
            .last_heard_at;
    assert!(
        silent < Duration::minutes(1),
        "a repeat of the held version is"
    );
}

#[tokio::test]
#[ignore]
async fn the_filter_read_denies_allows_and_surfaces_fail_open() {
    let Some(svc) = service().await else { return };
    let max_age = Duration::days(7);

    let never = Uuid::new_v4();
    assert_eq!(
        svc.standing_decision(never, max_age).await.unwrap(),
        StandingDecision::AllowUnheard
    );

    let good = Uuid::new_v4();
    svc.apply_account_standing(&standing(good, 1, true))
        .await
        .unwrap();
    assert_eq!(
        svc.standing_decision(good, max_age).await.unwrap(),
        StandingDecision::Allow
    );
    sqlx::query("UPDATE account_standing SET last_heard_at = now() - interval '8 days' WHERE account_id = $1")
        .bind(good)
        .execute(svc.pool())
        .await
        .unwrap();
    assert!(matches!(
        svc.standing_decision(good, max_age).await.unwrap(),
        StandingDecision::AllowStale { .. }
    ));

    let bad = Uuid::new_v4();
    svc.apply_account_standing(&standing(bad, 1, false))
        .await
        .unwrap();
    assert!(matches!(
        svc.standing_decision(bad, max_age).await.unwrap(),
        StandingDecision::Deny { .. }
    ));
}

#[tokio::test]
#[ignore]
async fn a_malformed_row_is_refused_by_the_table_itself() {
    let Some(svc) = service().await else { return };
    for (version, plan, url) in [
        (0i64, "p", None),
        (1, "", None),
        (1, "p", Some("javascript:alert(1)")),
        (1, "p", Some("http://insecure.example")),
    ] {
        let r = sqlx::query(
            "INSERT INTO account_standing (account_id, version, in_good_standing, plan_name, checkout_url) \
             VALUES ($1, $2, true, $3, $4)",
        )
        .bind(Uuid::new_v4())
        .bind(version)
        .bind(plan)
        .bind(url)
        .execute(svc.pool())
        .await;
        assert!(r.is_err(), "{version} {plan:?} {url:?} must be refused");
    }
}
