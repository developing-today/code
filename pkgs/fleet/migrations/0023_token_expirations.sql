-- 0023_token_expirations.sql — token / credential expiration + renewal
-- tracker (Drew, 2026-11-03: "in supabase"). One row per credential that
-- hard-expires or carries a rotation obligation. Values are NEVER stored
-- here — only the credential's name and where its value lives.

create table if not exists agent.token_expirations (
  id               bigint generated always as identity primary key,
  credential_name  text not null,
  service          text not null,
  purpose          text,
  scopes           text,
  issued_on        date,
  expires_on       date,          -- date-granularity expiry
  expires_at       timestamptz,   -- precise expiry when known (e.g. shared access tokens)
  renew_by         date,
  storage_location text,
  renewal_watch    text,          -- cron job id + fire date, when a watch exists
  status           text not null default 'active'
                   check (status in ('active', 'retired', 'check')),
  notes            text,
  created_at       timestamptz not null default now(),
  updated_at       timestamptz not null default now()
);

comment on table agent.token_expirations is
  'Credential expiration + renewal tracker. Names and storage locations only — never values.';

alter table agent.token_expirations enable row level security;

grant all on agent.token_expirations to service_role;
grant select, insert, update, delete on agent.token_expirations to authenticated;
grant select on agent.token_expirations to anon;
grant all on all sequences in schema agent to service_role;
