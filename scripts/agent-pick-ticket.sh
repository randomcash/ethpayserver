#!/bin/bash
# Resolve the next ticket for the autonomous worker and emit it to GITHUB_OUTPUT.
#
# Todo only. Backlog means "not ready" — a human decides what is ready, and that
# curation IS the queue. The old dispatcher read the same state but tracked what
# it had attempted in an append-only text ledger, which burned every ticket on
# first attempt and left it logging "idle" hourly for months (RCS-189). Linear's
# own state is the lock instead: Todo = available, In Progress = taken.
#
# Usage:  agent-pick-ticket.sh [RCS-123]
# Env:    LINEAR_API_KEY (required), GITHUB_OUTPUT (optional; prints if unset)

set -euo pipefail

TICKET_ARG="${1:-}"

if [ -z "${LINEAR_API_KEY:-}" ]; then
    echo "::error title=LINEAR_API_KEY not set::The worker cannot read the queue." >&2
    echo "Set it with: gh secret set LINEAR_API_KEY" >&2
    exit 1
fi

if [ -n "$TICKET_ARG" ]; then
    num="${TICKET_ARG#RCS-}"
    filter="{ team: { key: { eq: \\\"RCS\\\" } }, number: { eq: $num } }"
else
    filter="{ team: { key: { eq: \\\"RCS\\\" } }, state: { type: { eq: \\\"unstarted\\\" } } }"
fi

query="query { issues(filter: $filter, first: 1) { nodes { id identifier title description } } }"
resp=$(curl -fsS https://api.linear.app/graphql \
    -H "Authorization: $LINEAR_API_KEY" \
    -H "Content-Type: application/json" \
    --data "{\"query\":\"$query\"}")

if echo "$resp" | jq -e '.errors' >/dev/null 2>&1; then
    echo "::error title=Linear query failed::$(echo "$resp" | jq -c '.errors')" >&2
    exit 1
fi

count=$(echo "$resp" | jq '.data.issues.nodes | length')
out="${GITHUB_OUTPUT:-/dev/stdout}"

if [ "$count" -eq 0 ]; then
    echo "::notice title=Queue empty::No Todo tickets — nothing to do."
    echo "ticket=" >> "$out"
    exit 0
fi

ident=$(echo "$resp" | jq -r '.data.issues.nodes[0].identifier')
title=$(echo "$resp" | jq -r '.data.issues.nodes[0].title')

{
    echo "ticket=$ident"
    echo "title=$title"
    echo "body<<__TICKET_EOF__"
    echo "$resp" | jq -r '.data.issues.nodes[0].description // ""'
    echo "__TICKET_EOF__"
} >> "$out"

# Claim it. This is the lock: Todo = available, In Progress = taken. Without
# it two runs pick the same ticket — the local worker does not transition state
# at all, so RCS-177 sat in Todo after being completed and would be redone.
#
# Claiming BEFORE the work means a crashed run leaves the ticket In Progress
# rather than silently re-eligible. That is the safer failure: a stuck ticket is
# visible on the board, duplicated work is not.
claim() {
    local id="$1" state="$2"
    curl -fsS https://api.linear.app/graphql \
        -H "Authorization: $LINEAR_API_KEY" \
        -H "Content-Type: application/json" \
        --data "{\"query\":\"mutation { issueUpdate(id: \\\"$id\\\", input: { stateId: \\\"$state\\\" }) { success } }\"}"
}

if [ -z "${SKIP_CLAIM:-}" ]; then
    states=$(curl -fsS https://api.linear.app/graphql \
        -H "Authorization: $LINEAR_API_KEY" -H "Content-Type: application/json" \
        --data '{"query":"query { workflowStates(filter: { team: { key: { eq: \"RCS\" } }, type: { eq: \"started\" } }) { nodes { id name } } }"}')
    inprog=$(echo "$states" | jq -r '.data.workflowStates.nodes[] | select(.name=="In Progress") | .id' | head -1)
    uuid=$(echo "$resp" | jq -r '.data.issues.nodes[0].id // empty')
    if [ -n "$inprog" ] && [ -n "$uuid" ]; then
        if claim "$uuid" "$inprog" | jq -e '.data.issueUpdate.success' >/dev/null 2>&1; then
            echo "::notice title=Claimed::$ident moved to In Progress"
        else
            echo "::warning title=Claim failed::$ident stays Todo; a concurrent run may redo it"
        fi
    fi
fi

echo "::notice title=Picked $ident::$title"
