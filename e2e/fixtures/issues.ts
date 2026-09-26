// Labels that mark a known, structural test-infra limitation rather than
// something the suite is meant to catch - AUTH/REGISTER predate this file,
// COVERAGE_GAP covers the payment-detail check that E2E_REMOTE can't run
// (no DB to seed a payment against a live deployment). Every other label
// pushed through issue() fails the run - see scout.spec.ts's comment above
// its issue() function. Extracted so the gating behaviour itself has a test
// (scout-filter.spec.ts) independent of booting the whole app.
const NON_GATING_LABELS = ['AUTH', 'REGISTER', 'COVERAGE_GAP'];

export function gatingIssues(all: string[]): string[] {
  return all.filter((i) => !NON_GATING_LABELS.some((label) => i.startsWith(`[${label}]`)));
}
