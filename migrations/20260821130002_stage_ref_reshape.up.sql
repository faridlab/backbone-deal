-- Stage-driven opportunity lifecycle reshape.
--
-- Why: the sales pipeline is company configuration, not a fixed state list.
-- Opportunities now reference a configurable stages row (stage_id) instead of
-- the hardcoded sales_stage enum, won/lost derives from (stage.is_won,
-- probability, active) instead of being hand-set, and the pipeline carries
-- assignment (owner_user_id / sales_team_id) + stage-audit timestamps
-- (date_last_stage_update / date_closed). The preceding migration created
-- deal.stages; this one seeds it, backfills, and retires the enum.
--
-- Migrations run on the owner connection, which bypasses RLS (the module's
-- own fence-migration header notes this) — the seeded/backfilled rows are
-- written across all companies in one pass.
--
-- The default stage ids are DETERMINISTIC: uuid5(NAMESPACE_URL,
-- "deal-stage:{company_id}:{code}"). The lazy ensure-defaults in the
-- Opportunity repository computes the exact same ids in Rust — keep the
-- format string in sync with `ensure_default_stages` there (both call sites
-- carry this comment).

-- 0) uuid5 for the deterministic seed ids (contrib; trusted extension).
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- 1) New opportunity columns (nullable first; tightened after the backfill).
ALTER TABLE deal.opportunities ADD COLUMN IF NOT EXISTS stage_id UUID;
ALTER TABLE deal.opportunities ADD COLUMN IF NOT EXISTS date_last_stage_update TIMESTAMPTZ;
ALTER TABLE deal.opportunities ADD COLUMN IF NOT EXISTS date_closed TIMESTAMPTZ;
ALTER TABLE deal.opportunities ADD COLUMN IF NOT EXISTS active BOOLEAN NOT NULL DEFAULT TRUE;
ALTER TABLE deal.opportunities ADD COLUMN IF NOT EXISTS owner_user_id UUID;
ALTER TABLE deal.opportunities ADD COLUMN IF NOT EXISTS sales_team_id UUID;

-- 2) SEED the default stage set for every company that can hold deals:
--    every company already present on opportunities (static statement below),
--    plus every live company when the organization schema is installed (the
--    guarded DO block — fresh-DB deployments without organization skip that
--    branch; the guard is the same pg_namespace probe the catalog
--    tenant-scope migration uses). Deterministic ids make the seed a pure
--    function of (company_id, code): re-running converges and the opportunity
--    backfill below needs no join. Both statements are idempotent
--    (`ON CONFLICT DO NOTHING` on the (company_id, code) unique index), and
--    the second is wrapped in dynamic SQL because a static reference to
--    organization.companies would fail to parse where that schema is absent.
--
-- Entry probabilities are advisory hints carried over from the retiring
-- enum's implied funnel; stage moves never auto-apply them (callers send an
-- explicit probability when they want one). 'won' is the seeded is_won stage
-- — a stage, not an enum value, so companies can add their own won stages.
INSERT INTO deal.stages (id, company_id, code, name, sequence, is_won, probability_hint, active, metadata)
SELECT uuid_generate_v5(uuid_ns_url(), 'deal-stage:' || c.company_id || ':' || d.code),
       c.company_id, d.code, d.name, d.sequence, d.is_won, d.probability_hint, TRUE,
       '{"created_at":null,"updated_at":null,"deleted_at":null,"created_by":null,"updated_by":null,"deleted_by":null}'::jsonb
FROM (SELECT DISTINCT company_id FROM deal.opportunities) c
CROSS JOIN (VALUES ('prospecting','Prospecting',10,FALSE,10),
                   ('qualification','Qualification',20,FALSE,20),
                   ('proposal','Proposal',30,FALSE,40),
                   ('negotiation','Negotiation',40,FALSE,70),
                   ('closing','Closing',50,FALSE,90),
                   ('won','Won',60,TRUE,100)) AS d(code, name, sequence, is_won, probability_hint)
ON CONFLICT (company_id, code) DO NOTHING;

DO $seed_org$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = 'organization') THEN
        EXECUTE $q$
            INSERT INTO deal.stages (id, company_id, code, name, sequence, is_won, probability_hint, active, metadata)
            SELECT uuid_generate_v5(uuid_ns_url(), 'deal-stage:' || c.company_id || ':' || d.code),
                   c.company_id, d.code, d.name, d.sequence, d.is_won, d.probability_hint, TRUE,
                   '{"created_at":null,"updated_at":null,"deleted_at":null,"created_by":null,"updated_by":null,"deleted_by":null}'::jsonb
            FROM (SELECT id AS company_id FROM organization.companies
                  WHERE (metadata->>'deleted_at') IS NULL) c
            CROSS JOIN (VALUES ('prospecting','Prospecting',10,FALSE,10),
                               ('qualification','Qualification',20,FALSE,20),
                               ('proposal','Proposal',30,FALSE,40),
                               ('negotiation','Negotiation',40,FALSE,70),
                               ('closing','Closing',50,FALSE,90),
                               ('won','Won',60,TRUE,100)) AS d(code, name, sequence, is_won, probability_hint)
            ON CONFLICT (company_id, code) DO NOTHING
        $q$;
    END IF;
END
$seed_org$;

-- 3) BACKFILL opportunities from the retiring enum + status:
--    open rows land on their enum code's stage; won rows MUST sit on the
--    is_won stage (the derivation requires it) with probability normalized
--    to 100; lost rows keep their last pipeline stage (lost has no stage
--    requirement — the row remembers where it died) with probability 0.
UPDATE deal.opportunities o SET
    stage_id = CASE WHEN o.status = 'won'
                    THEN uuid_generate_v5(uuid_ns_url(), 'deal-stage:' || o.company_id || ':won')
                    ELSE uuid_generate_v5(uuid_ns_url(), 'deal-stage:' || o.company_id || ':' || o.sales_stage::text)
               END,
    active = (o.status <> 'lost'),
    probability = CASE WHEN o.status = 'won' THEN 100
                       WHEN o.status = 'lost' THEN 0
                       ELSE o.probability END,
    date_closed = CASE WHEN o.status IN ('won', 'lost') THEN NOW() END,
    date_last_stage_update = NOW();

-- 4) TIGHTEN: stage_id required, status/stage-shaped indexes replace the
--    sales_stage-shaped one, and row-local shape invariants gain DB CHECK
--    backstops (the cross-table "entering an is_won stage forces probability
--    100" rule is service-enforced — it is not CHECK-expressible).
ALTER TABLE deal.opportunities ALTER COLUMN stage_id SET NOT NULL;

DROP INDEX IF EXISTS deal.idx_opportunities_company_id_status_sales_stage;
CREATE INDEX IF NOT EXISTS idx_opportunities_company_id_status_stage_id
    ON deal.opportunities (company_id, status, stage_id);
CREATE INDEX IF NOT EXISTS idx_opportunities_stage_id
    ON deal.opportunities (stage_id);
CREATE INDEX IF NOT EXISTS idx_opportunities_owner_user_id
    ON deal.opportunities (owner_user_id);

ALTER TABLE deal.opportunities DROP CONSTRAINT IF EXISTS opportunities_probability_check;
ALTER TABLE deal.opportunities ADD CONSTRAINT opportunities_probability_range
    CHECK (probability >= 0 AND probability <= 100);
ALTER TABLE deal.opportunities ADD CONSTRAINT opportunities_won_shape
    CHECK (status <> 'won' OR (probability = 100 AND active AND date_closed IS NOT NULL));
ALTER TABLE deal.opportunities ADD CONSTRAINT opportunities_lost_shape
    CHECK (status <> 'lost' OR (probability = 0 AND NOT active AND date_closed IS NOT NULL));

-- 5) RETIRE the enum column + type. The type is referenced unqualified to
--    match how the original migration created it (schema-blind DO block):
--    the drop then resolves to the same type under whatever search_path the
--    runner used, instead of missing it.
ALTER TABLE deal.opportunities DROP COLUMN IF EXISTS sales_stage;
DROP TYPE IF EXISTS sales_stage;

-- 6) Strict company fence for the table introduced with this reshape
--    (the table-creation migration is generated and emits no RLS).
ALTER TABLE deal.stages ENABLE ROW LEVEL SECURITY;
ALTER TABLE deal.stages FORCE  ROW LEVEL SECURITY;
DROP POLICY IF EXISTS stages_company_isolation ON deal.stages;
CREATE POLICY stages_company_isolation ON deal.stages
    FOR ALL
    USING      (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid)
    WITH CHECK (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid);
