-- Down: drop deal.opportunity_items table
DROP TABLE IF EXISTS deal.opportunity_items CASCADE;
DROP FUNCTION IF EXISTS deal.opportunity_items_audit_timestamp() CASCADE;
