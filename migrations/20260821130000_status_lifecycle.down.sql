-- Down: restore the is_active boolean exactly as it was.
-- Only 'inactive' rows are written back as FALSE; rows at the column default
-- map to the boolean default TRUE without an UPDATE. The status-shaped index is
-- dropped with the status column; the original (company_id, is_active) index is
-- recreated by its original name.

ALTER TABLE deal.campaigns ADD COLUMN is_active BOOLEAN NOT NULL DEFAULT TRUE;
UPDATE deal.campaigns SET is_active = FALSE WHERE status = 'inactive';
ALTER TABLE deal.campaigns DROP COLUMN status;
CREATE INDEX IF NOT EXISTS idx_campaigns_company_id_is_active ON deal.campaigns (company_id, is_active);

DROP TYPE IF EXISTS campaign_status;
