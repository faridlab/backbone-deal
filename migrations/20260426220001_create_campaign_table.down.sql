-- Down: drop deal.campaigns table
DROP TABLE IF EXISTS deal.campaigns CASCADE;
DROP FUNCTION IF EXISTS deal.campaigns_audit_timestamp() CASCADE;
