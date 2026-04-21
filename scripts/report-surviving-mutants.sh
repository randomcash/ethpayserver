#!/usr/bin/env bash
# Parse cargo-mutants output and create Linear issues for surviving mutants.
# Called at the end of each mutation CI job.
#
# Usage: report-surviving-mutants.sh [mutants-output-dir]
# Requires: LINEAR_API_KEY env var (CI variable), curl, jq
#
# If LINEAR_API_KEY is unset, prints survivors to stdout and exits cleanly.
set -euo pipefail

MUTANTS_DIR="${1:-mutants.out}"
SURVIVED_FILE="${MUTANTS_DIR}/survived.txt"
TEAM_ID="df678d76-3cce-4ced-aa2e-9fdcfeb105a8"
LINEAR_API="https://api.linear.app/graphql"

if [ ! -f "$SURVIVED_FILE" ]; then
  echo "No survived.txt found — nothing to report."
  exit 0
fi

SURVIVED_COUNT=$(wc -l < "$SURVIVED_FILE")
if [ "$SURVIVED_COUNT" -eq 0 ]; then
  echo "All mutants caught by tests."
  exit 0
fi

echo "Found $SURVIVED_COUNT surviving mutant(s)."

if [ -z "${LINEAR_API_KEY:-}" ]; then
  echo "LINEAR_API_KEY not set — printing survivors only:"
  cat "$SURVIVED_FILE"
  exit 0
fi

# Ensure test-gap label exists on the RCS team.
get_or_create_label() {
  local existing
  existing=$(curl -sf -X POST "$LINEAR_API" \
    -H "Content-Type: application/json" \
    -H "Authorization: $LINEAR_API_KEY" \
    -d "$(jq -n --arg tid "$TEAM_ID" '{
      query: "query($tid: String!) { issueLabels(filter: { team: { id: { eq: $tid } }, name: { eq: \"test-gap\" } }) { nodes { id } } }",
      variables: { tid: $tid }
    }')" | jq -r '.data.issueLabels.nodes[0].id // empty')

  if [ -n "$existing" ]; then
    echo "$existing"
    return
  fi

  curl -sf -X POST "$LINEAR_API" \
    -H "Content-Type: application/json" \
    -H "Authorization: $LINEAR_API_KEY" \
    -d "$(jq -n --arg tid "$TEAM_ID" '{
      query: "mutation($tid: String!) { issueLabelCreate(input: { name: \"test-gap\", teamId: $tid }) { success issueLabel { id } } }",
      variables: { tid: $tid }
    }')" | jq -r '.data.issueLabelCreate.issueLabel.id // empty'
}

LABEL_ID=$(get_or_create_label)
if [ -z "$LABEL_ID" ]; then
  echo "WARNING: could not get/create test-gap label, filing issues without it."
fi

# File one issue per surviving mutant line.
CREATED=0
while IFS= read -r line; do
  [ -z "$line" ] && continue

  # survived.txt lines look like:
  #   server/src/api/invoices.rs:42: replace foo -> bool with true
  title="Test gap: ${line:0:200}"

  body="## Surviving Mutant

\`\`\`
$line
\`\`\`

This mutation was applied and **no test caught it**. The code change compiled
and all tests still passed, meaning this code path lacks adequate test coverage.

Source: weekly \`cargo-mutants\` run ([RCS-111](https://linear.app/randomcash/issue/RCS-111))."

  label_array="[]"
  if [ -n "$LABEL_ID" ]; then
    label_array="[\"$LABEL_ID\"]"
  fi

  result=$(curl -sf -X POST "$LINEAR_API" \
    -H "Content-Type: application/json" \
    -H "Authorization: $LINEAR_API_KEY" \
    -d "$(jq -n \
      --arg title "$title" \
      --arg body "$body" \
      --arg tid "$TEAM_ID" \
      --argjson labels "$label_array" \
      '{
        query: "mutation($input: IssueCreateInput!) { issueCreate(input: $input) { success issue { identifier url } } }",
        variables: { input: { title: $title, description: $body, teamId: $tid, priority: 3, labelIds: $labels } }
      }')" 2>/dev/null || echo '{}')

  issue_id=$(echo "$result" | jq -r '.data.issueCreate.issue.identifier // "FAILED"')
  echo "  Created $issue_id for: $line"
  CREATED=$((CREATED + 1))
done < "$SURVIVED_FILE"

echo "Filed $CREATED Linear issue(s) tagged test-gap."
