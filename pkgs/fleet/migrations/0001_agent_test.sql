insert into agent.tasks (queue, title, body, dedupe_key, priority) values
 ('meta','Agent schema live in Supabase','Coordination schema v1 (Postgres) built 2026-10-20 alongside the Turso coord v1; Postgres is the designated authoritative home for exactly-one-owner state. Turso copy stays as mirror until cutover.','agent-schema-live-2026-10-20', 10),
 ('research','Test task: claim/complete cycle','Synthetic task used to prove claim_task/complete_task.', 'test-claim-cycle', 50),
 ('research','Test task: fail and requeue','Synthetic task used to prove fail_task requeue.', 'test-fail-requeue', 60)
on conflict (dedupe_key) do nothing;
select (agent.claim_task('sentinel','meta',900)).id as sentinel_claimed;
select (agent.claim_task('rnd','research',900)).id as rnd_claimed_first;
select (agent.complete_task(id,'rnd','{"proof":"claim-complete cycle ok"}'::jsonb)).status as rnd_completed
  from agent.tasks where dedupe_key='test-claim-cycle';
select (agent.claim_task('engineering','research',900)).id as eng_claimed;
select (agent.fail_task(id,'engineering','synthetic failure for requeue proof', true)).status as after_fail
  from agent.tasks where dedupe_key='test-fail-requeue';
update agent.tasks set status='done', completed_at=now(), worker_id='muse', result='{"note":"schema build task closed by coordinator"}' where dedupe_key='agent-schema-live-2026-10-20';
select id, queue, title, status, worker_id, attempts from agent.tasks order by id;
select worker_id, kind, count(*) from agent.events group by 1,2 order by 1,2;
select count(*) as leases_remaining from agent.leases;
