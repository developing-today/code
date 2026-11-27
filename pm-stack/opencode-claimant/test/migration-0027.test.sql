-- Focused tests for migration-0027 (#16066): the error-stopped
-- recovery mode in agent.bind_repo_task_lane_session.
--
-- Scenarios only; the minimal agent schema and the agent.test_seed
-- helper live in migration-0027.schema.sql. Every scenario asserts
-- with the oracle named in its comment; a failed assertion raises
-- and fails the whole run (ON_ERROR_STOP).

-- S1 (oracle: the migration-0022/0026 recovery contract, applied to
-- the error-stopped marker): error-stopped recovery after lease
-- expiry atomically clears BOTH bindings, consumes one attempt,
-- records launch_failed with the error-stopped reason, and emits
-- the lane_session_recovered event.
do $$
begin
  perform agent.test_seed('-10 minutes', 1, 4);
  perform agent.bind_repo_task_lane_session(
    1, 'opencode-claimant', 'ses_old',
    'opencode-claimant-recovery:error-stopped');
  if (select session_id from agent.repo_task_lane_ledger where task_id = 1) is not null then
    raise exception 'TEST FAIL S1: ledger binding not cleared';
  end if;
  if (select progress ->> 'session_id' from agent.tasks where id = 1) is not null
     or (select progress ->> 'stage' from agent.tasks where id = 1) <> 'launch_failed'
     or (select progress ->> 'launch_failed_session_id' from agent.tasks where id = 1) <> 'ses_old'
     or (select progress ->> 'launch_failed_reason' from agent.tasks where id = 1)
        <> 'verified error-stopped session deleted before recovery' then
    raise exception 'TEST FAIL S1: task progress does not record the recovery';
  end if;
  if (select attempts from agent.tasks where id = 1) <> 2 then
    raise exception 'TEST FAIL S1: recovery did not consume one attempt';
  end if;
  if not exists (select 1 from agent.events where task_id = 1 and kind = 'lane_session_recovered') then
    raise exception 'TEST FAIL S1: lane_session_recovered event missing';
  end if;
end $$;

-- S2 (oracle: compare-and-clear guard): a mismatched prior session
-- id refuses the error-stopped recovery and leaves BOTH stores
-- unchanged.
do $$
declare v_msg text;
begin
  perform agent.test_seed('-10 minutes', 1, 4);
  begin
    perform agent.bind_repo_task_lane_session(
      1, 'opencode-claimant', 'ses_other',
      'opencode-claimant-recovery:error-stopped');
    raise exception 'TEST FAIL S2: mismatched recovery was accepted';
  exception when others then
    v_msg := sqlerrm;
    if v_msg like 'TEST FAIL%' then raise; end if;
    if v_msg not like '%recovery binding mismatch%' then
      raise exception 'TEST FAIL S2: unexpected error: %', v_msg;
    end if;
  end;
  if (select session_id from agent.repo_task_lane_ledger where task_id = 1) <> 'ses_old'
     or (select progress ->> 'session_id' from agent.tasks where id = 1) <> 'ses_old'
     or (select attempts from agent.tasks where id = 1) <> 1 then
    raise exception 'TEST FAIL S2: a store changed after a refused recovery';
  end if;
end $$;

-- S3 (oracle: the recovery lease decision): error-stopped recovery
-- against a LIVE lease is refused - a stale claimant can never
-- clear a binding whose replacement may still legitimately launch.
do $$
declare v_msg text;
begin
  perform agent.test_seed('+10 minutes', 1, 4);
  begin
    perform agent.bind_repo_task_lane_session(
      1, 'opencode-claimant', 'ses_old',
      'opencode-claimant-recovery:error-stopped');
    raise exception 'TEST FAIL S3: live-lease recovery was accepted';
  exception when others then
    v_msg := sqlerrm;
    if v_msg like 'TEST FAIL%' then raise; end if;
    if v_msg not like '%requires an expired lease%' then
      raise exception 'TEST FAIL S3: unexpected error: %', v_msg;
    end if;
  end;
  if (select session_id from agent.repo_task_lane_ledger where task_id = 1) <> 'ses_old' then
    raise exception 'TEST FAIL S3: ledger binding changed after a refused recovery';
  end if;
end $$;

-- S4 (oracle: attempts accounting): error-stopped recovery with
-- attempts exhausted is refused and clears nothing.
do $$
declare v_msg text;
begin
  perform agent.test_seed('-10 minutes', 4, 4);
  begin
    perform agent.bind_repo_task_lane_session(
      1, 'opencode-claimant', 'ses_old',
      'opencode-claimant-recovery:error-stopped');
    raise exception 'TEST FAIL S4: exhausted recovery was accepted';
  exception when others then
    v_msg := sqlerrm;
    if v_msg like 'TEST FAIL%' then raise; end if;
    if v_msg not like '%recovery attempts exhausted%' then
      raise exception 'TEST FAIL S4: unexpected error: %', v_msg;
    end if;
  end;
  if (select session_id from agent.repo_task_lane_ledger where task_id = 1) <> 'ses_old' then
    raise exception 'TEST FAIL S4: ledger binding changed after a refused recovery';
  end if;
end $$;
