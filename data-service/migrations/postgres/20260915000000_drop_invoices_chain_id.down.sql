-- Restores the column in its pre-drop shape: nullable caip2, always empty.
ALTER TABLE invoices ADD COLUMN chain_id caip2;
