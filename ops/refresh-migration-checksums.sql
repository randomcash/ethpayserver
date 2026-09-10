-- Refresh `_sqlx_migrations` checksums after the ticket-id sweep.
--
-- The sweep edited comments inside migration files that are already applied.
-- sqlx checksums the whole file - SHA-384 of its bytes, see `Migration::new` in
-- sqlx-core - and refuses to run when a stored checksum no longer matches
-- (`migrator.rs`, VersionMismatch). So `migrate` would fail on the next deploy
-- even though not one statement changed, and `server` would never start behind
-- it.
--
-- Nothing here alters schema or data. It re-records the checksum, and the
-- description sqlx now derives from the filename, for migrations that have
-- already run. Against a database where they have not run, every UPDATE matches
-- nothing and this is a no-op.
--
-- ============================================================================
-- THIS IS ONE-WAY. Read before running.
-- ============================================================================
--
-- `sqlx::migrate!` bakes the checksums into the binary at compile time, so a
-- previously built image carries the OLD ones. Once this script has run, that
-- image's `migrate` exits with VersionMismatch and `server` never starts behind
-- it - which means **rolling back to any image built before this change stops
-- working**, and rollback is otherwise just a `compose up` with an older tag.
--
-- Run it only together with deploying an image built from a tree that contains
-- the edited migrations. To go back afterwards you must restore the previous
-- checksums as well; take the rows first:
--
--   psql "$DATABASE_URL" -c "\copy (SELECT version, encode(checksum,'hex') \
--     FROM _sqlx_migrations ORDER BY version) TO 'checksums-before.csv' CSV"
--
-- Run it in EVERY environment whose database has these migrations applied, not
-- just the one you are deploying now. Nine of them date back to April 2026, so
-- any long-lived database has them. Nothing in CI runs this for you.
--
-- Look first:
--   psql "$DATABASE_URL" -c 'SELECT version, description FROM _sqlx_migrations ORDER BY version'
--
-- Then (no -1: the transaction is below, and ON_ERROR_STOP makes a failure
-- exit non-zero instead of reporting success over a rolled-back batch):
--   psql "$DATABASE_URL" -f ops/refresh-migration-checksums.sql
--
-- The values below were verified three ways: recomputed from the files, checked
-- against what the embedded `sqlx::migrate!` migrator reports, and compared
-- against the 17 untouched rows in the live testnet database, which match
-- byte-for-byte.

\set ON_ERROR_STOP on

BEGIN;

UPDATE _sqlx_migrations
   SET checksum = '\xcf5992975535bce31da81a64051e7e8ddf9f3fc9c5938b9aa9d75d71c45fe927e08d51a061500b3fe5ea44b5945c52e7'::bytea,
       description = 'create auth tables'
 WHERE version = 20241214000001;

UPDATE _sqlx_migrations
   SET checksum = '\xeb172caed3623cf5efa6104c17ebb42d5cfe4478112721c87d4d5e9d32f694121422574c538d4e7358a1481f82728626'::bytea,
       description = 'create payment tables'
 WHERE version = 20241214000002;

UPDATE _sqlx_migrations
   SET checksum = '\x1bcbbb87b1ccec75c7ec5b7db08441fe78d34cc88169368c578dde7d4602d524dffc0c602a504f9ca0d47a13d796d844'::bytea,
       description = 'add store wallet'
 WHERE version = 20241215000001;

UPDATE _sqlx_migrations
   SET checksum = '\xe105a3396624763652855125bd4009b77a278499ffd5bb0550cde5582ab4f72e0661806841c5886950091b61486db9df'::bytea,
       description = 'add store webhooks'
 WHERE version = 20241216000001;

UPDATE _sqlx_migrations
   SET checksum = '\x71c2efafba384d137469b86658d35c9d047a31a5c4d952323b0caeab2cf2e13266d99f8e47b968992051db685115b255'::bytea,
       description = 'add store payment methods'
 WHERE version = 20241228000001;

UPDATE _sqlx_migrations
   SET checksum = '\xb747cc187ea3c17c88c5d083a1b4d22032a2b27c7e75c455b9e35765832ba9853e241ae0768651caababe0b03dc6676f'::bytea,
       description = 'add payment aggregation'
 WHERE version = 20241229000001;

UPDATE _sqlx_migrations
   SET checksum = '\x495d6617015bc308afbcc7600584797c36e13a3dfa8f1578451ad25e6970a9b259eb6a16336fee9261a2166e5ca71520'::bytea,
       description = 'add discoverable auth challenges'
 WHERE version = 20260118000001;

UPDATE _sqlx_migrations
   SET checksum = '\x1f240049b8fb8ad5a92851a30eb213b2587e7808ac4c308f239580b7f754b7a577c42f1f0d38984a03e00fc0bbf00a6d'::bytea,
       description = 'allow passkey only users'
 WHERE version = 20260118000002;

UPDATE _sqlx_migrations
   SET checksum = '\x946a8a935cb376e28d23ce6320f256642607ae5c41b9d2bb3e72abd601f96b88010e9c104221063a5c9f3e28506fa8af'::bytea,
       description = 'create api keys'
 WHERE version = 20260403000001;

UPDATE _sqlx_migrations
   SET checksum = '\xb7498930c2ec38c7a7daebbddf78cbb3cdc9f81b07b32c3b50fec88c3a19e4ddf651f26325114d58d1733cacc5e9cf90'::bytea,
       description = 'add composite indexes'
 WHERE version = 20260413000001;

UPDATE _sqlx_migrations
   SET checksum = '\x55e6c736b78314da37c6107eaa4185b55d6ee1ca87e4892579252dcc4d21792561c728ac504113a29bbf8e010c04d84e'::bytea,
       description = 'fix invoice expiration'
 WHERE version = 20260413000002;

UPDATE _sqlx_migrations
   SET checksum = '\xa1fff054aad464d1b333d10a4e28fa3c0ae11fe1d6bbc7b77520b0b9757d5b60a259fa443ae3a6175d758c3f8bd3d18f'::bytea,
       description = 'add fantom gnosis chains'
 WHERE version = 20260415000001;

UPDATE _sqlx_migrations
   SET checksum = '\xa4ab0017a829709db397160b8a240378249c20c5578bc974ec6f86f674a8dba616383becfcd34a18de6a004b4f99ebed'::bytea,
       description = 'create refunds and payouts'
 WHERE version = 20260416000001;

UPDATE _sqlx_migrations
   SET checksum = '\xb6f8efd927d9b5dad527610fb5a1e005ccd726246f7e47eeb539318ed5f2fb6948606b36699aa48bcf077e99a6fe643b'::bytea,
       description = 'api key rate limit'
 WHERE version = 20260418000001;

UPDATE _sqlx_migrations
   SET checksum = '\x380e4a76744c693e70076344499767f854cbe011d1cdcd97cdf98a7fe8f97fbbbe2c302546e79c96f3bf3cb3cd62ab54'::bytea,
       description = 'create webhook deliveries'
 WHERE version = 20260418000002;

UPDATE _sqlx_migrations
   SET checksum = '\x183b0b86a3c89380e53d8e7bd6f3910971949863978e95e09e9df37781039c57bca8720126cf8170dffccc29a20bd9ce'::bytea,
       description = 'create store settings'
 WHERE version = 20260420000001;

UPDATE _sqlx_migrations
   SET checksum = '\x425637bd84e95c17554d877ec740edd187dac4acc9c966fe7ad93f25b1115b2da04c2cedddbe09eb47c6c8c4b9c43624'::bytea,
       description = 'api key deprecation'
 WHERE version = 20260423000001;

UPDATE _sqlx_migrations
   SET checksum = '\x06f1f2317726e9132fc9e39212c12a9a4bd8be41b6e49c9758c07814a863a5e08ab20b82a61794017f609afb9a1e9647'::bytea,
       description = 'wallet rotations'
 WHERE version = 20260425011418;

UPDATE _sqlx_migrations
   SET checksum = '\xec472ac9d7ca0d402f500655704c4545430e96c68ac1fb52a746bbd9444c774b9148882dc284f1b92336fef1b2789f6c'::bytea,
       description = 'customer email and receipts'
 WHERE version = 20260425120000;

UPDATE _sqlx_migrations
   SET checksum = '\x962051f3ff86a4c5fa4625386d8a1645890afdbb5877913416dedb2e22af2bbc802e8ae751e2112299100bc5f8f8c883'::bytea,
       description = 'store token policies'
 WHERE version = 20260505100000;

UPDATE _sqlx_migrations
   SET checksum = '\x4ecef1c23a2159f5dad6819c54bb0582b181480716e051368e87e1d7e3e30e1f66d3ea9bef9bfa9bcc3e94c49096e44a'::bytea,
       description = 'create server settings'
 WHERE version = 20260505120000;

UPDATE _sqlx_migrations
   SET checksum = '\x0b1bc139b4bb3e3f9c39826a213308dc863a17ff82080aecf93ae0459ac438f839836e549e82e2895badef0b91d34d64'::bytea,
       description = 'add kdf salt identifier'
 WHERE version = 20260823150000;

UPDATE _sqlx_migrations
   SET checksum = '\x0d1b5310c0bae5aefdd78cedb54853ef660c99dffa1eb5fad2ab0273535afd8e18b3f8a925bffc0de41b727c7864d04c'::bytea,
       description = 'backfill kdf salt identifier'
 WHERE version = 20260823150100;

UPDATE _sqlx_migrations
   SET checksum = '\xa76303b9677996c21391d17bb3335651b5e1fa0d49acd1a381e76d4cfda31143da37b6be9efc3472f65e6fc813d8879a'::bytea,
       description = 'customer email column'
 WHERE version = 20260906150000;

UPDATE _sqlx_migrations
   SET checksum = '\x23f8ad4367c7298b5e7e768675c4b3a1671ae0e284ec8a6d2cc7f5481d57b06c7ce5169b25b17bfadbbe24f3098c4861'::bytea,
       description = 'account wallets'
 WHERE version = 20260908120000;

UPDATE _sqlx_migrations
   SET checksum = '\x7c408d2f0902d74543e15cf533b5120782d072b0207304398070b5f8323481790972a3f766902b113b8d23ea93adf632'::bytea,
       description = 'caip2 chain identity'
 WHERE version = 20260908140000;

COMMIT;
