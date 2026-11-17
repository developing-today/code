-- 0005_grants.sql — PostgREST role grants for the custom schemas.
-- Supabase grants these automatically in `public` only. Custom schemas
-- need: USAGE for the API roles, table/sequence/function privileges for
-- service_role (RLS-bypassing admin), and limited grants for
-- anon/authenticated (RLS still applies — with no policies they see nothing).

do $$
declare s text;
begin
  foreach s in array array['agent','ci','ideas','ainventory'] loop
    execute format('grant usage on schema %I to anon, authenticated, service_role', s);
    execute format('grant all on all tables in schema %I to service_role', s);
    execute format('grant select, insert, update, delete on all tables in schema %I to authenticated', s);
    execute format('grant select on all tables in schema %I to anon', s);
    execute format('grant all on all sequences in schema %I to service_role', s);
    execute format('grant usage, select on all sequences in schema %I to authenticated, anon', s);
    execute format('grant execute on all functions in schema %I to service_role, authenticated', s);
    execute format('alter default privileges in schema %I grant all on tables to service_role', s);
    execute format('alter default privileges in schema %I grant all on sequences to service_role', s);
  end loop;
end $$;
