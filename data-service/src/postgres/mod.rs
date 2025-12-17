//! PostgreSQL implementation of the repository traits.

use sqlx::postgres::PgPool;

mod auth;
mod conversions;
mod invoice;
mod payment;
mod token;
mod watched_address;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod integration_tests;

/// PostgreSQL data service implementation.
#[derive(Clone)]
pub struct PgDataService {
    pool: PgPool,
}

impl PgDataService {
    /// Create a new PgDataService with the given connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Get a reference to the connection pool.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}
