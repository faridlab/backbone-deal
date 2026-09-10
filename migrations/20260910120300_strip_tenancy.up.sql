-- Hand-authored (user-owned). Not regenerated.
--
-- Strip every company-fence artifact from the deal tables (ADR-0029): the module is
-- tenant-agnostic; org scoping is installed by the COMPOSING service's tenancy decorator,
-- never by the module. Dropped here, per table: the company-leading indexes, the
-- <table>_company_isolation RLS policy, and the company_id column itself.
--
-- Tables: campaigns, opportunities, opportunity_items, stages.
--
-- Ordering guard (the decorator must run FIRST on any database with data): the module
-- never moves tenancy data. A table is safe to strip when EITHER
--   a) it carries org_unit_id with no NULLs — the decorator backfilled it from company_id —
--      or b) it is empty (a fresh database: the earlier chain files created it empty).
-- Otherwise the strip RAISEs, naming the decorator step, rather than dropping a column
-- that still holds the only tenancy key. The file is re-runnable (every drop is IF EXISTS
-- and the tracker has no checksums), so a failed run retries cleanly after the decorator
-- lands.
--
-- RLS enable/force flags are deliberately NOT touched: the decorator owns those now.
-- The tenant-free domain artifacts stay: every remaining index (opportunities on
-- lead_id / party_id / campaign_id / stage_id / owner_user_id, opportunity_items on
-- opportunity_id) and the stage-shape CHECKs key no tenant column. The pipeline
-- uniqueness posture (one stage code/name per unit) is owned by the composing
-- decorator's per-unit uniques and is intentionally NOT restored here — the pre-fence
-- global forms would forbid two units of one tenant from keeping their own pipelines.

DO $$
DECLARE
    t text;
    has_org boolean;
    org_nulls bigint;
    total bigint;
    offenders text := '';
BEGIN
    FOREACH t IN ARRAY ARRAY['campaigns', 'opportunities', 'opportunity_items', 'stages']
    LOOP
        IF to_regclass(format('deal.%I', t)) IS NULL THEN
            CONTINUE; -- chain not fully applied on this database; nothing to strip
        END IF;

        SELECT EXISTS (
                   SELECT 1 FROM information_schema.columns
                   WHERE table_schema = 'deal' AND table_name = t AND column_name = 'org_unit_id'
               )
        INTO has_org;

        EXECUTE format('SELECT count(*) FROM deal.%I', t) INTO total;

        IF has_org THEN
            EXECUTE format(
                'SELECT count(*) FROM deal.%I WHERE org_unit_id IS NULL', t)
            INTO org_nulls;
        ELSE
            org_nulls := total; -- no org column: every row's only tenancy key is company_id
        END IF;

        IF has_org AND org_nulls = 0 THEN
            CONTINUE; -- decorator backfilled: safe
        END IF;
        IF total = 0 THEN
            CONTINUE; -- empty table (fresh database): safe
        END IF;
        offenders := offenders || format(' deal.%s (%s rows, %s rows not covered by org_unit_id);', t, total, org_nulls);
    END LOOP;

    IF offenders <> '' THEN
        RAISE EXCEPTION 'refusing to strip company_id — these tables are not yet covered by the tenancy decorator:%. Apply the composing service''s tenancy decorator (it backfills org_unit_id from company_id) and re-run; it is the only step that moves tenancy data.', offenders;
    END IF;
END $$;

-- ── campaigns ─────────────────────────────────────────────────────────────────
DROP INDEX IF EXISTS deal.idx_campaigns_company_id_is_active;
DROP INDEX IF EXISTS deal.idx_campaigns_company_id_status;
DROP POLICY IF EXISTS campaigns_company_isolation ON deal.campaigns;
ALTER TABLE deal.campaigns DROP COLUMN IF EXISTS company_id;

-- ── opportunities ─────────────────────────────────────────────────────────────
DROP INDEX IF EXISTS deal.idx_opportunities_company_id_status_sales_stage;
DROP INDEX IF EXISTS deal.idx_opportunities_company_id_status_stage_id;
DROP POLICY IF EXISTS opportunities_company_isolation ON deal.opportunities;
ALTER TABLE deal.opportunities DROP COLUMN IF EXISTS company_id;

-- ── opportunity_items ─────────────────────────────────────────────────────────
DROP POLICY IF EXISTS opportunity_items_company_isolation ON deal.opportunity_items;
ALTER TABLE deal.opportunity_items DROP COLUMN IF EXISTS company_id;

-- ── stages ────────────────────────────────────────────────────────────────────
DROP INDEX IF EXISTS deal.idx_stages_company_id_code;
DROP INDEX IF EXISTS deal.idx_stages_company_id_name;
DROP INDEX IF EXISTS deal.idx_stages_company_id_sequence;
DROP POLICY IF EXISTS stages_company_isolation ON deal.stages;
ALTER TABLE deal.stages DROP COLUMN IF EXISTS company_id;
