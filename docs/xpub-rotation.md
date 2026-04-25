# XPub Rotation Runbook

Rotate a store's extended public key (xpub) when a key is compromised, a
developer machine is breached, or as part of a scheduled key rotation policy.

## When to rotate

- **Key compromise**: xpub exposed in logs, backups, or a stolen device.
- **Personnel change**: developer who had access leaves the team.
- **Scheduled rotation**: periodic rotation as part of security hygiene.

## What happens during rotation

1. **All payment methods** for the store are updated to the new xpub.
2. **Derivation indices reset** to zero on each payment method.
3. **Existing watched addresses** remain active. In-flight invoices (pending,
   processing) continue to detect payments on old-xpub-derived addresses.
4. **New invoices** derive payment addresses from the new xpub.
5. **A rotation record** is persisted per payment method for audit.

Old addresses are only deactivated when their parent invoice resolves (paid,
expired, or cancelled). No manual cleanup is needed.

## API

```
POST /stores/{store_id}/wallet/rotate
Authorization: Bearer <token>
Content-Type: application/json

{
  "xpub": "<new-xpub>",
  "reason": "key compromise"   // optional
}
```

### Response (200 OK)

```json
{
  "store_id": "uuid",
  "new_xpub_masked": "xpub6CUG...3fDVmz",
  "methods_rotated": 3,
  "rotations": [
    {
      "id": "uuid",
      "payment_method_id": "uuid",
      "chain_id": 11155111,
      "asset_symbol": "ETH",
      "previous_xpub_masked": "xpub6D4B...cLW5",
      "previous_derivation_index": 42,
      "rotated_at": "2026-04-25T01:14:18Z"
    }
  ]
}
```

### Error codes

| Code | Meaning |
|------|---------|
| 400  | Invalid xpub, or all methods already use the provided xpub |
| 403  | User lacks `canmodifystoresettings` permission |
| 404  | Store not found or no payment methods configured |

## Procedure

### 1. Generate a new xpub

Use your wallet software (e.g., MetaMask, Trezor Suite, Ledger Live) to export
a fresh BIP-32 extended public key. The key must be a valid `xpub` (base58).

### 2. Rotate via API

```bash
curl -X POST https://pay.random.cash/stores/<STORE_ID>/wallet/rotate \
  -H "Authorization: Bearer <TOKEN>" \
  -H "Content-Type: application/json" \
  -d '{"xpub": "<NEW_XPUB>", "reason": "scheduled rotation"}'
```

### 3. Verify

- Create a test invoice and confirm its payment address derives from the new
  xpub (compare the first derived address at index 0 with your wallet).
- Check that any in-flight invoices still accept payments on their existing
  (old-xpub) addresses.

### 4. Revoke old key material

- Remove the old xpub from any backups, `.env` files, or password managers.
- If the old xpub was a hardware wallet account, consider disabling that
  account path to prevent accidental reuse.

## Reversal

Rotation is reversible: call the same endpoint with the original xpub. The
derivation index resets to zero, which means previously-used indices will be
re-derived. This is safe because address reuse in a receive-only context does
not leak funds, but it may confuse payment reconciliation. Only reverse if the
rotation was a mistake.

## Audit trail

Rotation history is stored in the `wallet_rotations` table. Each row records:

- The previous and new xpub
- Which payment method was rotated
- The derivation index at rotation time
- Optional reason string
- Timestamp
