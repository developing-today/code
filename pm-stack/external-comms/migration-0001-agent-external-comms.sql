-- External-comms send-time ledger (Drew ratified 2026-11-05).
-- Rule: memory/topics/external-comms-rule-2026-11-05.md; design:
-- pm-stack/external-comms/README.md in developing-today/code.
-- Fleet ops data: lives in the `agent` schema beside tasks /
-- quota_records (canonical project muse-dev2).

create table if not exists agent.external_comms (
  id                text primary key,          -- stable record id; pusher retries dedupe on it
  sent_at           timestamptz not null,      -- when the send happened
  recorded_at       timestamptz not null default now(),
  identity          text not null,             -- 'drew' | 'fleet:<name>' (e.g. fleet:openclaw)
  channel           text not null,             -- github | gmail | slack | facebook | ...
  destination       text not null,             -- repo / addressee / channel the send went to
  item_ref          text,                      -- URL of the item, when one exists
  kind              text not null,             -- issue | pr | comment | review | email | post | dm
  approval_basis    text not null,             -- per-item approval | surfaced-with-link | allowlist:<name> | honest backfill labels
  approver          text,                      -- who approved, when approval applies
  approval_evidence text,                      -- where the approval is recorded (chat/memory/issue ref)
  summary           text not null,             -- one line: what was sent
  payload           jsonb not null default '{}'::jsonb,  -- channel detail (slack ts, case no, provenance)
  source            text not null default 'record-external-send'  -- writer: CLI | backfill-2026-11-05-audit
);

create index if not exists external_comms_sent_at_idx
  on agent.external_comms (sent_at desc);
create index if not exists external_comms_channel_idx
  on agent.external_comms (channel, sent_at desc);

alter table agent.external_comms enable row level security;
grant select, insert, update, delete on agent.external_comms to service_role;

comment on table agent.external_comms is
  'Send-time ledger for external comms under Drew''s or a fleet identity (rule ratified 2026-11-05). Writers: record-external-send CLI on the laptop (DB-first, spill to local outbox, pusher drains).';
