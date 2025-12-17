//! Integration tests for auth repository implementations.
//! Require DATABASE_URL environment variable.
//! Run with: DATABASE_URL="postgres://..." cargo test -p data-service -- --ignored

use chrono::{Duration, Utc};

use auth::{
    ChallengeRepository, DeviceRepository, DeviceType, Role, Session, SessionId, SessionRepository,
    User, UserId, UserRepository, WalletCredentialId, WalletRepository,
    error::AuthError,
};

use super::PgDataService;
use super::tests::{
    test_device, test_encrypted_blob, test_kdf_params, test_session, test_user, test_wallet,
    test_wallet_challenge,
};

async fn create_test_service() -> Option<PgDataService> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .ok()?;
    Some(PgDataService::new(pool))
}

fn unique_email() -> String {
    format!("test_{}@example.com", uuid::Uuid::new_v4())
}

fn unique_wallet_address() -> String {
    format!("0x{:040x}", uuid::Uuid::new_v4().as_u128())
}

#[tokio::test]
#[ignore]
async fn integration_user_crud() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create user
    let mut user = test_user();
    user.email = Some(unique_email());
    service.create_user(&user).await.unwrap();

    // Get user by ID
    let fetched = service.get_user(user.id).await.unwrap().unwrap();
    assert_eq!(fetched.id, user.id);
    assert_eq!(fetched.email, user.email);

    // Get user by email
    let fetched = service
        .get_user_by_email(user.email.as_ref().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.id, user.id);

    // Update user
    let mut updated = fetched;
    updated.failed_login_attempts = 3;
    service.update_user(&updated).await.unwrap();

    let fetched = service.get_user(user.id).await.unwrap().unwrap();
    assert_eq!(fetched.failed_login_attempts, 3);

    // Increment failed logins
    let count = service.increment_failed_logins(user.id).await.unwrap();
    assert_eq!(count, 4);

    // Reset failed logins
    service.reset_failed_logins(user.id).await.unwrap();
    let fetched = service.get_user(user.id).await.unwrap().unwrap();
    assert_eq!(fetched.failed_login_attempts, 0);

    // Lock user
    let lock_until = Utc::now() + Duration::hours(1);
    service.lock_user(user.id, lock_until).await.unwrap();
    let fetched = service.get_user(user.id).await.unwrap().unwrap();
    assert!(fetched.is_locked());

    // Unlock user
    service.unlock_user(user.id).await.unwrap();
    let fetched = service.get_user(user.id).await.unwrap().unwrap();
    assert!(!fetched.is_locked());

    // Delete user
    service.delete_user(user.id).await.unwrap();
    let fetched = service.get_user(user.id).await.unwrap();
    assert!(fetched.is_none());
}

#[tokio::test]
#[ignore]
async fn integration_user_wallet_address() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create user with wallet address
    let user = User {
        id: UserId::new(),
        email: None,
        primary_wallet_address: Some(unique_wallet_address()),
        kdf_params: test_kdf_params(),
        encrypted_symmetric_key: test_encrypted_blob(),
        recovery_verification_hash: "hash".to_string(),
        created_at: Utc::now(),
        last_login_at: None,
        failed_login_attempts: 0,
        locked_until: None,
        role: Role::User,
    };
    service.create_user(&user).await.unwrap();

    // Get by wallet address
    let fetched = service
        .get_user_by_wallet_address(user.primary_wallet_address.as_ref().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.id, user.id);

    // Cleanup
    service.delete_user(user.id).await.unwrap();
}

#[tokio::test]
#[ignore]
async fn integration_device_crud() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create user first
    let mut user = test_user();
    user.email = Some(unique_email());
    service.create_user(&user).await.unwrap();

    // Create device
    let device = test_device(user.id);
    service.create_device(&device).await.unwrap();

    // Get device
    let fetched = service.get_device(device.id).await.unwrap().unwrap();
    assert_eq!(fetched.id, device.id);
    assert_eq!(fetched.user_id, user.id);
    assert_eq!(fetched.device_type, DeviceType::Browser);

    // Get devices for user
    let devices = service.get_devices_for_user(user.id).await.unwrap();
    assert_eq!(devices.len(), 1);

    // Count active devices
    let count = service.count_active_devices(user.id).await.unwrap();
    assert_eq!(count, 1);

    // Deactivate device
    service.deactivate_device(device.id).await.unwrap();
    let fetched = service.get_device(device.id).await.unwrap().unwrap();
    assert!(!fetched.is_active);

    // Count should be 0 now
    let count = service.count_active_devices(user.id).await.unwrap();
    assert_eq!(count, 0);

    // Delete device
    service.delete_device(device.id).await.unwrap();
    let fetched = service.get_device(device.id).await.unwrap();
    assert!(fetched.is_none());

    // Cleanup
    service.delete_user(user.id).await.unwrap();
}

#[tokio::test]
#[ignore]
async fn integration_session_crud() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create user and device first
    let mut user = test_user();
    user.email = Some(unique_email());
    service.create_user(&user).await.unwrap();

    let device = test_device(user.id);
    service.create_device(&device).await.unwrap();

    // Create session
    let session = test_session(user.id, device.id);
    service.create_session(&session).await.unwrap();

    // Get session
    let fetched = service.get_session(session.id).await.unwrap().unwrap();
    assert_eq!(fetched.id, session.id);
    assert_eq!(fetched.user_id, user.id);
    assert_eq!(fetched.device_id, device.id);
    // PostgreSQL INET returns CIDR notation
    assert!(fetched.ip_address.as_ref().unwrap().starts_with("127.0.0.1"));

    // Get sessions for user
    let sessions = service.get_sessions_for_user(user.id).await.unwrap();
    assert_eq!(sessions.len(), 1);

    // Update session
    let mut updated = fetched;
    updated.ip_address = Some("192.168.1.1".to_string());
    service.update_session(&updated).await.unwrap();

    let fetched = service.get_session(session.id).await.unwrap().unwrap();
    assert!(fetched.ip_address.as_ref().unwrap().starts_with("192.168.1.1"));

    // Delete session
    service.delete_session(session.id).await.unwrap();
    let fetched = service.get_session(session.id).await.unwrap();
    assert!(fetched.is_none());

    // Cleanup
    service.delete_user(user.id).await.unwrap();
}

#[tokio::test]
#[ignore]
async fn integration_session_cascade_delete() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create user and device
    let mut user = test_user();
    user.email = Some(unique_email());
    service.create_user(&user).await.unwrap();

    let device = test_device(user.id);
    service.create_device(&device).await.unwrap();

    // Create multiple sessions
    let session1 = test_session(user.id, device.id);
    let session2 = Session {
        id: SessionId::new(),
        ..test_session(user.id, device.id)
    };
    service.create_session(&session1).await.unwrap();
    service.create_session(&session2).await.unwrap();

    // Delete all sessions for user
    service.delete_all_sessions_for_user(user.id).await.unwrap();
    let sessions = service.get_sessions_for_user(user.id).await.unwrap();
    assert!(sessions.is_empty());

    // Cleanup
    service.delete_user(user.id).await.unwrap();
}

#[tokio::test]
#[ignore]
async fn integration_wallet_crud() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create user first
    let mut user = test_user();
    user.email = Some(unique_email());
    service.create_user(&user).await.unwrap();

    // Create wallet
    let mut wallet = test_wallet(user.id);
    wallet.address = unique_wallet_address();
    service.create_wallet(&wallet).await.unwrap();

    // Get wallet
    let fetched = service.get_wallet(wallet.id).await.unwrap().unwrap();
    assert_eq!(fetched.id, wallet.id);
    assert_eq!(fetched.address, wallet.address);

    // Get wallet by address
    let fetched = service
        .get_wallet_by_address(&wallet.address)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.id, wallet.id);

    // Get wallets for user
    let wallets = service.get_wallets_for_user(user.id).await.unwrap();
    assert_eq!(wallets.len(), 1);

    // Count active wallets
    let count = service.count_active_wallets(user.id).await.unwrap();
    assert_eq!(count, 1);

    // Deactivate wallet
    service.deactivate_wallet(wallet.id).await.unwrap();
    let fetched = service.get_wallet(wallet.id).await.unwrap().unwrap();
    assert!(!fetched.is_active);

    // Deactivated wallet should not be found by address
    let fetched = service.get_wallet_by_address(&wallet.address).await.unwrap();
    assert!(fetched.is_none());

    // Delete wallet
    service.delete_wallet(wallet.id).await.unwrap();
    let fetched = service.get_wallet(wallet.id).await.unwrap();
    assert!(fetched.is_none());

    // Cleanup
    service.delete_user(user.id).await.unwrap();
}

#[tokio::test]
#[ignore]
async fn integration_wallet_unique_constraint() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create two users
    let mut user1 = test_user();
    user1.email = Some(unique_email());
    service.create_user(&user1).await.unwrap();

    let mut user2 = test_user();
    user2.id = UserId::new();
    user2.email = Some(unique_email());
    service.create_user(&user2).await.unwrap();

    // Create wallet for user1
    let address = unique_wallet_address();
    let mut wallet1 = test_wallet(user1.id);
    wallet1.address = address.clone();
    service.create_wallet(&wallet1).await.unwrap();

    // Try to create wallet with same address for user2
    let mut wallet2 = test_wallet(user2.id);
    wallet2.id = WalletCredentialId::new();
    wallet2.address = address;
    let result = service.create_wallet(&wallet2).await;
    assert!(matches!(result, Err(AuthError::WalletAlreadyRegistered)));

    // Cleanup
    service.delete_user(user1.id).await.unwrap();
    service.delete_user(user2.id).await.unwrap();
}

#[tokio::test]
#[ignore]
async fn integration_wallet_challenge() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    let user_id = UserId::new();
    let challenge = test_wallet_challenge();

    // Store challenge
    service
        .store_wallet_challenge(user_id, challenge.clone())
        .await
        .unwrap();

    // Take challenge
    let fetched = service.take_wallet_challenge(user_id).await.unwrap().unwrap();
    assert_eq!(fetched.challenge, challenge.challenge);
    assert_eq!(fetched.address, challenge.address);

    // Challenge should be gone after taking
    let fetched = service.take_wallet_challenge(user_id).await.unwrap();
    assert!(fetched.is_none());
}

#[tokio::test]
#[ignore]
async fn integration_cascade_delete_user() {
    let service = create_test_service().await.expect("DATABASE_URL required");

    // Create user with device, session, and wallet
    let mut user = test_user();
    user.email = Some(unique_email());
    service.create_user(&user).await.unwrap();

    let device = test_device(user.id);
    service.create_device(&device).await.unwrap();

    let session = test_session(user.id, device.id);
    service.create_session(&session).await.unwrap();

    let mut wallet = test_wallet(user.id);
    wallet.address = unique_wallet_address();
    service.create_wallet(&wallet).await.unwrap();

    // Delete user - should cascade to all related records
    service.delete_user(user.id).await.unwrap();

    // Verify all related records are gone
    assert!(service.get_device(device.id).await.unwrap().is_none());
    assert!(service.get_session(session.id).await.unwrap().is_none());
    assert!(service.get_wallet(wallet.id).await.unwrap().is_none());
}
