-- Down: remove the company RLS fence for deal module

-- Reverse the company RLS fence for deal.campaigns
DROP POLICY IF EXISTS campaigns_company_isolation ON deal.campaigns;
ALTER TABLE deal.campaigns NO FORCE ROW LEVEL SECURITY;
ALTER TABLE deal.campaigns DISABLE ROW LEVEL SECURITY;

-- Reverse the company RLS fence for deal.opportunities
DROP POLICY IF EXISTS opportunities_company_isolation ON deal.opportunities;
ALTER TABLE deal.opportunities NO FORCE ROW LEVEL SECURITY;
ALTER TABLE deal.opportunities DISABLE ROW LEVEL SECURITY;

-- Reverse the company RLS fence for deal.opportunity_items
DROP POLICY IF EXISTS opportunity_items_company_isolation ON deal.opportunity_items;
ALTER TABLE deal.opportunity_items NO FORCE ROW LEVEL SECURITY;
ALTER TABLE deal.opportunity_items DISABLE ROW LEVEL SECURITY;

