-- 0003_ideas.sql — mirror of Drew's idea registers. Self-contained in schema `ideas`.
-- The Markdown registers remain the source of truth; this is a queryable
-- mirror (dedupe_key = source + section path + title), refreshed by the
-- mirror script in ~/workspace/supabase/idea-mirror/.

create schema if not exists ideas;

create table if not exists ideas.sources (
  id          bigint generated always as identity primary key,
  name        text not null unique,
  kind        text not null default 'markdown-register',
  path        text,
  note        text,
  mirrored_at timestamptz
);

create table if not exists ideas.ideas (
  id          bigint generated always as identity primary key,
  source_id   bigint not null references ideas.sources(id),
  title       text not null,
  body        text,
  section     text,
  status      text not null default 'captured',
  tags        text[] not null default '{}',
  source_ref  text,
  dedupe_key  text not null unique,
  created_at  timestamptz not null default now(),
  mirrored_at timestamptz not null default now()
);
create index if not exists ideas_source_status_idx on ideas.ideas (source_id, status);

alter table ideas.sources enable row level security;
alter table ideas.ideas   enable row level security;
