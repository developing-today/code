begin;
do $$
declare
  slug text;
  t agent.tasks;
  slugs text[] := array[
    'developing-today/code',
    'developing-today-forks/opencode',
    'developing-today-forks/openchamber',
    'dezren39/darwin-nix',
    'developing-today/zettel',
    'dezren39/dezren39',
    'unrelated/some-repo'
  ];
begin
  foreach slug in array slugs loop
    begin
      t := agent.enqueue_repo_task_fleet(
        'repo-health-code-pilot',
        'task 15126 regression probe (rolled back)',
        null,
        jsonb_build_object('schema','repo-task/v1','repo', slug, 'kind','triage'),
        'task15126-probe:' || slug,
        50, null, 1);
      raise notice 'ACCEPT % -> task %', slug, t.id;
    exception when others then
      raise notice 'REJECT % -> %', slug, sqlerrm;
    end;
  end loop;
end $$;
rollback;
select count(*) as probe_rows_remaining from agent.tasks where dedupe_key like 'task15126-probe:%';
