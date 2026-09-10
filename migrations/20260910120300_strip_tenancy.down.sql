-- Hand-authored (user-owned). Not regenerated.
--
-- Best-effort restore sketch for the tenancy strip (ADR-0029). This is a breaking module
-- release against dev-stage databases: the down re-adds the company_id column as nullable
-- with the company isolation policy shape, but restores NO data — rows written after the
-- strip (or after the decorator re-keyed them) carry org_unit_id only. The composing
-- service's tenancy decorator remains the live fence; treat this down as a schema-shape
-- sketch for archaeology, not a usable rollback.

ALTER TABLE deal.campaigns          ADD COLUMN IF NOT EXISTS company_id uuid;
ALTER TABLE deal.opportunities      ADD COLUMN IF NOT EXISTS company_id uuid;
ALTER TABLE deal.opportunity_items  ADD COLUMN IF NOT EXISTS company_id uuid;
ALTER TABLE deal.stages             ADD COLUMN IF NOT EXISTS company_id uuid;

CREATE INDEX IF NOT EXISTS idx_campaigns_company_id_status
    ON deal.campaigns (company_id, status);
CREATE INDEX IF NOT EXISTS idx_opportunities_company_id_status_stage_id
    ON deal.opportunities (company_id, status, stage_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_stages_company_id_code
    ON deal.stages (company_id, code);
CREATE UNIQUE INDEX IF NOT EXISTS idx_stages_company_id_name
    ON deal.stages (company_id, name);
CREATE INDEX IF NOT EXISTS idx_stages_company_id_sequence
    ON deal.stages (company_id, sequence);

CREATE POLICY campaigns_company_isolation ON deal.campaigns
    FOR ALL
    USING      (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid)
    WITH CHECK (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid);
CREATE POLICY opportunities_company_isolation ON deal.opportunities
    FOR ALL
    USING      (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid)
    WITH CHECK (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid);
CREATE POLICY opportunity_items_company_isolation ON deal.opportunity_items
    FOR ALL
    USING      (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid)
    WITH CHECK (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid);
CREATE POLICY stages_company_isolation ON deal.stages
    FOR ALL
    USING      (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid)
    WITH CHECK (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid);
