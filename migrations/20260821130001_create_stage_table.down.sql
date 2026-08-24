-- Down: drop deal.stages table
DROP TABLE IF EXISTS deal.stages CASCADE;
DROP FUNCTION IF EXISTS deal.stages_audit_timestamp() CASCADE;
