-- Down for the stage_ref reshape — LOSSY BY DESIGN (documented, like the
-- recruitment stage reshape before it):
--   * the seeded default stages are not removed (the create_stage_table down
--     drops the table wholesale);
--   * won rows that were lost-to-history beyond "won" collapse back onto the
--     'closing' enum value — the 'won' stage code has no enum counterpart;
--   * date_last_stage_update / date_closed / owner_user_id / sales_team_id
--     are dropped with their columns.
-- Status restores inversely from the derivation: lost ⇐ inactive + 0,
-- won ⇐ active + 100 + an is_won stage, open otherwise.

-- The backstop CHECKs go first: the inverse status map writes rows the
-- reshape-time CHECKs would reject.
ALTER TABLE deal.opportunities DROP CONSTRAINT IF EXISTS opportunities_probability_range;
ALTER TABLE deal.opportunities DROP CONSTRAINT IF EXISTS opportunities_won_shape;
ALTER TABLE deal.opportunities DROP CONSTRAINT IF EXISTS opportunities_lost_shape;

DROP INDEX IF EXISTS deal.idx_opportunities_company_id_status_stage_id;
DROP INDEX IF EXISTS deal.idx_opportunities_stage_id;
DROP INDEX IF EXISTS deal.idx_opportunities_owner_user_id;

-- Recreate the enum column (schema-blind DO block, matching how the original
-- create migration emitted it) and map back from the stage rows.
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'sales_stage') THEN
        CREATE TYPE sales_stage AS ENUM ('prospecting', 'qualification', 'proposal', 'negotiation', 'closing');
    END IF;
END
$$;

ALTER TABLE deal.opportunities ADD COLUMN IF NOT EXISTS sales_stage sales_stage NOT NULL DEFAULT 'prospecting';

UPDATE deal.opportunities o SET
    sales_stage = CASE WHEN s.code = 'won' THEN 'closing'::sales_stage
                       ELSE s.code::sales_stage END,
    status = CASE WHEN (NOT o.active AND o.probability = 0) THEN 'lost'::opportunity_status
                  WHEN (o.active AND o.probability = 100 AND s.is_won) THEN 'won'::opportunity_status
                  ELSE 'open'::opportunity_status END
FROM deal.stages s
WHERE s.id = o.stage_id;

CREATE INDEX IF NOT EXISTS idx_opportunities_company_id_status_sales_stage
    ON deal.opportunities (company_id, status, sales_stage);

-- Drop the columns this reshape introduced.
ALTER TABLE deal.opportunities DROP COLUMN IF EXISTS stage_id;
ALTER TABLE deal.opportunities DROP COLUMN IF EXISTS date_last_stage_update;
ALTER TABLE deal.opportunities DROP COLUMN IF EXISTS date_closed;
ALTER TABLE deal.opportunities DROP COLUMN IF EXISTS active;
ALTER TABLE deal.opportunities DROP COLUMN IF EXISTS owner_user_id;
ALTER TABLE deal.opportunities DROP COLUMN IF EXISTS sales_team_id;

-- Drop the fence this reshape armed (the table itself is dropped by the
-- create_stage_table down migration).
DROP POLICY IF EXISTS stages_company_isolation ON deal.stages;
ALTER TABLE deal.stages NO FORCE ROW LEVEL SECURITY;
ALTER TABLE deal.stages DISABLE ROW LEVEL SECURITY;
