-- Migration: replace the campaign lifecycle boolean with a status enum
-- campaigns carried `is_active BOOLEAN NOT NULL DEFAULT TRUE`; the tree-wide convention is one
-- `status` enum field per lifecycle (see docs/refactoring-schema in the serpa workspace).
-- The boolean migrates only rows deviating from its own column default; the dependent
-- (company_id, is_active) index is dropped with the column and replaced by a status-shaped one.
-- The enum type is created unqualified so it lands beside the module's other enum types (public),
-- where the generated sqlx type_name resolves.

DO $$ BEGIN
    CREATE TYPE campaign_status AS ENUM ('active', 'inactive');
EXCEPTION WHEN duplicate_object THEN NULL; END $$;

ALTER TABLE deal.campaigns ADD COLUMN status campaign_status NOT NULL DEFAULT 'active';
UPDATE deal.campaigns SET status = 'inactive' WHERE NOT is_active;
ALTER TABLE deal.campaigns DROP COLUMN is_active;
CREATE INDEX IF NOT EXISTS idx_campaigns_company_id_status ON deal.campaigns (company_id, status);
