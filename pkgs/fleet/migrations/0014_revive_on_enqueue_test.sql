-- 0014 synthetic test (live DB, shadow queue, synthetic dedupe key):
--   enqueue -> enqueue (live) returns the existing row UNCHANGED
--   enqueue -> cancel -> enqueue returns a LIVE queued task, same row id,
--   with an 'enqueued' event carrying revived_from_cancelled: true.
-- Run AFTER applying 0014_revive_on_enqueue.sql. Raises on any failure.
-- Leaves the synthetic task queued as evidence; the operator cancels it
-- as cleanup after reading the evidence (see #1298 closing comment).

-- 1. enqueue (new)
select (agent.enqueue_repo_task(
  'repo-health-code-shadow', 'Synthetic revive test v1 (#1298)', 'test body',
  '{"schema":"repo-task/v1","repo":"developing-today/code","kind":"triage"}'::jsonb,
  'repo-health:v1:test:revive-on-enqueue:1298', 42, null, 3)).id as step1_id;

-- 2. enqueue again on the live row with different title/priority:
--    must return the existing row unchanged.
select (agent.enqueue_repo_task(
  'repo-health-code-shadow', 'Synthetic revive test CHANGED TITLE', 'changed',
  '{"schema":"repo-task/v1","repo":"developing-today/code","kind":"triage"}'::jsonb,
  'repo-health:v1:test:revive-on-enqueue:1298', 99, null, 3)).id as step2_id;

do $$
declare t agent.tasks;
begin
  select * into t from agent.tasks
   where dedupe_key = 'repo-health:v1:test:revive-on-enqueue:1298';
  if t.status <> 'queued' then
    raise exception 'step2: expected queued, got %', t.status; end if;
  if t.title <> 'Synthetic revive test v1 (#1298)' or t.priority <> 42 then
    raise exception 'step2: live row was modified by duplicate enqueue (title=%, priority=%)',
      t.title, t.priority; end if;
end $$;

-- 3. cancel it (operator verb), then enqueue again: revival.
select (agent.cancel_repo_task(id, 'repo-worker-code',
        'synthetic test for #1298 revive-on-enqueue')).status as step3_cancelled
  from agent.tasks
 where dedupe_key = 'repo-health:v1:test:revive-on-enqueue:1298';

select (agent.enqueue_repo_task(
  'repo-health-code-shadow', 'Synthetic revive test v2 REVIVED (#1298)', 'revived body',
  '{"schema":"repo-task/v1","repo":"developing-today/code","kind":"triage"}'::jsonb,
  'repo-health:v1:test:revive-on-enqueue:1298', 17, null, 3)).id as step3_revived_id;

do $$
declare t agent.tasks; ev agent.events;
begin
  select * into t from agent.tasks
   where dedupe_key = 'repo-health:v1:test:revive-on-enqueue:1298';
  if t.status <> 'queued' then
    raise exception 'step3: expected revived row queued, got %', t.status; end if;
  if t.title <> 'Synthetic revive test v2 REVIVED (#1298)' or t.priority <> 17 then
    raise exception 'step3: revived row does not carry the new packet (title=%, priority=%)',
      t.title, t.priority; end if;
  if t.attempts <> 0 or t.result is not null or t.last_error is not null
     or t.worker_id is not null or t.lease_until is not null then
    raise exception 'step3: revived row not reset (attempts=%, result=%, last_error=%)',
      t.attempts, t.result, t.last_error; end if;
  select * into ev from agent.events
   where task_id = t.id and kind = 'enqueued'
     and detail ->> 'revived_from_cancelled' = 'true'
   order by id desc limit 1;
  if not found then
    raise exception 'step3: no enqueued event with revived_from_cancelled=true'; end if;
end $$;

-- Evidence read: the row and its full event trail.
select id, queue, status, title, priority, attempts from agent.tasks
 where dedupe_key = 'repo-health:v1:test:revive-on-enqueue:1298';
select task_id, kind, detail from agent.events
 where task_id = (select id from agent.tasks
                   where dedupe_key = 'repo-health:v1:test:revive-on-enqueue:1298')
 order by id;
