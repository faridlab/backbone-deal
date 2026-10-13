-- Down: drop the declared foreign keys for deal module
ALTER TABLE IF EXISTS deal.opportunity_items DROP CONSTRAINT IF EXISTS fk_opportunity_items_opportunity_id;
ALTER TABLE IF EXISTS deal.opportunities DROP CONSTRAINT IF EXISTS fk_opportunities_campaign_id;
ALTER TABLE IF EXISTS deal.opportunities DROP CONSTRAINT IF EXISTS fk_opportunities_stage_id;
