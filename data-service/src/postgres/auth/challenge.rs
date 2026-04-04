//! ChallengeRepository implementation.
//! Uses separate tables per challenge type as defined in migrations.

use async_trait::async_trait;
use sqlx::Row;

use auth::{
    ChallengeRepository, DiscoverableAuthentication, PasskeyAuthentication, PasskeyRegistration,
    UserId, WalletChallenge,
    error::{AuthError, Result},
};

use super::{PgDataService, sqlx_to_auth_error};

#[async_trait]
impl ChallengeRepository for PgDataService {
    async fn store_registration_challenge(
        &self,
        user_id: UserId,
        email: &str,
        state: PasskeyRegistration,
    ) -> Result<()> {
        let state_json =
            serde_json::to_value(&state).map_err(|e| AuthError::Repository(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO passkey_registration_challenges (user_id, identifier, challenge_state, created_at)
            VALUES ($1, $2, $3, NOW())
            ON CONFLICT (user_id) DO UPDATE SET
                identifier = EXCLUDED.identifier,
                challenge_state = EXCLUDED.challenge_state,
                created_at = NOW()
            "#,
        )
        .bind(user_id.0)
        .bind(email)
        .bind(&state_json)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        Ok(())
    }

    async fn take_registration_challenge(
        &self,
        user_id: UserId,
    ) -> Result<Option<(PasskeyRegistration, String)>> {
        // Challenge expires after 5 minutes
        let row = sqlx::query(
            r#"
            DELETE FROM passkey_registration_challenges
            WHERE user_id = $1 AND created_at > NOW() - INTERVAL '10 minutes'
            RETURNING challenge_state, identifier
            "#,
        )
        .bind(user_id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        match row {
            Some(r) => {
                let state_json: serde_json::Value = r.get("challenge_state");
                let identifier: String = r.get("identifier");
                let state: PasskeyRegistration = serde_json::from_value(state_json)
                    .map_err(|e| AuthError::Repository(e.to_string()))?;
                Ok(Some((state, identifier)))
            }
            None => Ok(None),
        }
    }

    async fn store_authentication_challenge(
        &self,
        user_id: UserId,
        state: PasskeyAuthentication,
    ) -> Result<()> {
        let state_json =
            serde_json::to_value(&state).map_err(|e| AuthError::Repository(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO passkey_authentication_challenges (user_id, challenge_state, created_at)
            VALUES ($1, $2, NOW())
            ON CONFLICT (user_id) DO UPDATE SET
                challenge_state = EXCLUDED.challenge_state,
                created_at = NOW()
            "#,
        )
        .bind(user_id.0)
        .bind(&state_json)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        Ok(())
    }

    async fn take_authentication_challenge(
        &self,
        user_id: UserId,
    ) -> Result<Option<PasskeyAuthentication>> {
        let row = sqlx::query(
            r#"
            DELETE FROM passkey_authentication_challenges
            WHERE user_id = $1 AND created_at > NOW() - INTERVAL '10 minutes'
            RETURNING challenge_state
            "#,
        )
        .bind(user_id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        match row {
            Some(r) => {
                let state_json: serde_json::Value = r.get("challenge_state");
                let state: PasskeyAuthentication = serde_json::from_value(state_json)
                    .map_err(|e| AuthError::Repository(e.to_string()))?;
                Ok(Some(state))
            }
            None => Ok(None),
        }
    }

    async fn store_discoverable_authentication_challenge(
        &self,
        challenge_id: uuid::Uuid,
        state: DiscoverableAuthentication,
    ) -> Result<()> {
        let state_json =
            serde_json::to_value(&state).map_err(|e| AuthError::Repository(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO discoverable_authentication_challenges (challenge_id, challenge_state, created_at)
            VALUES ($1, $2, NOW())
            ON CONFLICT (challenge_id) DO UPDATE SET
                challenge_state = EXCLUDED.challenge_state,
                created_at = NOW()
            "#,
        )
        .bind(challenge_id)
        .bind(&state_json)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        Ok(())
    }

    async fn take_discoverable_authentication_challenge(
        &self,
        challenge_id: uuid::Uuid,
    ) -> Result<Option<DiscoverableAuthentication>> {
        let row = sqlx::query(
            r#"
            DELETE FROM discoverable_authentication_challenges
            WHERE challenge_id = $1 AND created_at > NOW() - INTERVAL '10 minutes'
            RETURNING challenge_state
            "#,
        )
        .bind(challenge_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        match row {
            Some(r) => {
                let state_json: serde_json::Value = r.get("challenge_state");
                let state: DiscoverableAuthentication = serde_json::from_value(state_json)
                    .map_err(|e| AuthError::Repository(e.to_string()))?;
                Ok(Some(state))
            }
            None => Ok(None),
        }
    }

    async fn store_wallet_challenge(
        &self,
        user_id: UserId,
        challenge: WalletChallenge,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO wallet_challenges (user_id, challenge, address, created_at)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (user_id) DO UPDATE SET
                challenge = EXCLUDED.challenge,
                address = EXCLUDED.address,
                created_at = EXCLUDED.created_at
            "#,
        )
        .bind(user_id.0)
        .bind(&challenge.challenge)
        .bind(&challenge.address)
        .bind(challenge.created_at)
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        Ok(())
    }

    async fn take_wallet_challenge(&self, user_id: UserId) -> Result<Option<WalletChallenge>> {
        let row = sqlx::query(
            r#"
            DELETE FROM wallet_challenges
            WHERE user_id = $1 AND created_at > NOW() - INTERVAL '10 minutes'
            RETURNING challenge, address, created_at
            "#,
        )
        .bind(user_id.0)
        .fetch_optional(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        match row {
            Some(r) => Ok(Some(WalletChallenge {
                challenge: r.get("challenge"),
                address: r.get("address"),
                created_at: r.get("created_at"),
            })),
            None => Ok(None),
        }
    }

    async fn cleanup_expired_challenges(&self) -> Result<u64> {
        let r1 = sqlx::query(
            "DELETE FROM passkey_registration_challenges WHERE created_at < NOW() - INTERVAL '10 minutes'"
        )
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        let r2 = sqlx::query(
            "DELETE FROM passkey_authentication_challenges WHERE created_at < NOW() - INTERVAL '10 minutes'"
        )
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        let r3 = sqlx::query(
            "DELETE FROM wallet_challenges WHERE created_at < NOW() - INTERVAL '10 minutes'",
        )
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        let r4 = sqlx::query(
            "DELETE FROM discoverable_authentication_challenges WHERE created_at < NOW() - INTERVAL '10 minutes'"
        )
        .execute(&self.pool)
        .await
        .map_err(sqlx_to_auth_error)?;

        Ok(r1.rows_affected() + r2.rows_affected() + r3.rows_affected() + r4.rows_affected())
    }
}
