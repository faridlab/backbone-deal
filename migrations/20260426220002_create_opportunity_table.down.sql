-- Down: drop deal.opportunities table
DROP TABLE IF EXISTS deal.opportunities CASCADE;
DROP FUNCTION IF EXISTS deal.opportunities_audit_timestamp() CASCADE;
