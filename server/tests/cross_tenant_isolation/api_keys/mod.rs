//! API keys: the same tenancy boundary, reached through the other auth path
//!
//! The stored row (id, owner, name, hash, prefix, active flag, expiry) has no
//! scope field narrower than "everything its owner can do" - there is no
//! per-key store binding and no permission set to shrink. So "a key carrying
//! more than its owner's scope" has two shapes this schema can actually
//! produce, and this module covers both: a bearer-token path that resolves
//! to the wrong owner, or skips the per-request tenant check a session goes
//! through (`scope_parity` and `cross_tenant_reads`, re-running every
//! session-tenancy assertion through `authenticate_via_bearer` - the real
//! key-hash lookup and owner resolution, not a hand-built session); and a
//! key the active/expiry check should have already refused authenticating
//! anyway, carrying its owner's full access past a point it should never
//! have reached (`lifecycle`, which goes through
//! `AuthenticatedUser::from_request_parts` directly to assert the rejection
//! itself, not just what a successful call can reach). If a narrower per-key
//! scope is ever added, it needs its own tests here.
//!
//! What this module does NOT claim: it is not a reproduction of any specific
//! tracked defect, open or otherwise - this file has no way to read a tracker
//! and doesn't try to. It is an exhaustive list of the scope-violation shapes
//! *this schema* can structurally produce today. If a real key-scope bug
//! turns out to need a shape this schema cannot express (e.g. a key legitimately
//! narrower than its owner, reaching beyond that narrower grant), no test here
//! proves or disproves it, and closing that would need a schema change first -
//! a new column and a check against it, then a test here for that check.

mod cross_tenant_reads;
mod lifecycle;
mod scope_parity;
