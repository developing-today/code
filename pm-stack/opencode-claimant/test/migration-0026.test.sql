-- Focused tests for migration-0026 (#5400): the restored guarded
-- recovery modes in agent.bind_repo_task_lane_session.
--
-- Scenarios only; the minimal agent schema and the agent.test_seed
-- helper live in migration-0026.schema.sql. Every scenario asserts
-- with the oracle named in its comment; a failed assertion raises
-- and fails the whole run (ON_ERROR_STOP).

-- S1 (oracle: the migration-0022 recovery contract): zero-message
-- recovery after lease expiry atomically clears BOTH bindings,
-- consumes one attempt, records launch_failed, and emits the event.
do $$
begin
  perform agent.test_seed('-10 minutes', 1, 4);
  perform agent.bind_repo_task_lane_session(
    1, 'opencode-claimant', 'ses_old',
    'opencode-claimant-recovery:zero-message-deleted');
  if (select session_id from agent.repo_task_lane_ledger where task_id = 1) is not null then
    raise exception 'TEST FAIL S1: ledger binding not cleared';
  end if;
  if (select progress ->> 'session_id' from agent.tasks where id = 1) is not null
     or (select progress ->> 'stage' from agent.tasks where id = 1) <> 'launch_failed'
     or (select progress ->> 'launch_failed_session_id' from agent.tasks where id = 1) <> 'ses_old'
     or (select progress ->> 'launch_failed_reason' from agent.tasks where id = 1)
        <> 'verified zero-message session deleted before recovery' then
    raise exception 'TEST FAIL S1: task progress does not record the recovery';
  end if;
  if (select attempts from agent.tasks where id = 1) <> 2 then
    raise exception 'TEST FAIL S1: recovery did not consume one attempt';
  end if;
  if not exists (select 1 from agent.events where task_id = 1 and kind = 'lane_session_recovered') then
    raise exception 'TEST FAIL S1: lane_session_recovered event missing';
  end if;
end $$;

-- S2 (oracle: same contract, gone-session reason text).
do $$
begin
  perform agent.test_seed('-10 minutes', 0, 4);
  perform agent.bind_repo_task_lane_session(
    1, 'opencode-claimant', 'ses_old',
    'opencode-claimant-recovery:gone-session');
  if (select progress ->> 'launch_failed_reason' from agent.tasks where id = 1)
     <> 'bound session verified absent before recovery' then
    raise exception 'TEST FAIL S2: gone-session reason not recorded';
  end if;
  if (select session_id from agent.repo_task_lane_ledger where task_id = 1) is not null then
    raise exception 'TEST FAIL S2: ledger binding not cleared';
  end if;
end $$;

-- S3 (oracle: compare-and-clear guard): a mismatched prior session
-- id refuses and leaves BOTH stores byte-identical.
do $$
declare v_msg text;
begin
  perform agent.test_seed('-10 minutes', 1, 4);
  begin
    perform agent.bind_repo_task_lane_session(
      1, 'opencode-claimant', 'ses_other',
      'opencode-claimant-recovery:gone-session');
    raise exception 'TEST FAIL S3: mismatched recovery was accepted';
  exception when others then
    v_msg := sqlerrm;
    if v_msg like 'TEST FAIL%' then raise; end if;
    if v_msg not like '%recovery binding mismatch%' then
      raise exception 'TEST FAIL S3: wrong refusal: %', v_msg;
    end if;
  end;
  if (select session_id from agent.repo_task_lane_ledger where task_id = 1) <> 'ses_old'
     or (select progress ->> 'session_id' from agent.tasks where id = 1) <> 'ses_old'
     or (select attempts from agent.tasks where id = 1) <> 1 then
    raise exception 'TEST FAIL S3: a store changed on a refused recovery';
  end if;
end $$;

-- S4 (oracle: the #5400 lease decision — prompt.md invokes recovery
-- only after expiry): recovery against a LIVE lease refuses and
-- changes nothing.
do $$
declare v_msg text;
begin
  perform agent.test_seed('+10 minutes', 1, 4);
  begin
    perform agent.bind_repo_task_lane_session(
      1, 'opencode-claimant', 'ses_old',
      'opencode-claimant-recovery:zero-message-deleted');
    raise exception 'TEST FAIL S4: recovery over a live lease was accepted';
  exception when others then
    v_msg := sqlerrm;
    if v_msg like 'TEST FAIL%' then raise; end if;
    if v_msg not like '%requires an expired lease%' then
      raise exception 'TEST FAIL S4: wrong refusal: %', v_msg;
    end if;
  end;
  if (select session_id from agent.repo_task_lane_ledger where task_id = 1) <> 'ses_old' then
    raise exception 'TEST FAIL S4: ledger changed on a refused recovery';
  end if;
end $$;

-- S5 (oracle: recovery is the claimant's operation alone).
do $$
declare v_msg text;
begin
  perform agent.test_seed('-10 minutes', 1, 4);
  begin
    perform agent.bind_repo_task_lane_session(
      1, 'repo-worker-code', 'ses_old',
      'opencode-claimant-recovery:gone-session');
    raise exception 'TEST FAIL S5: recovery by a non-claimant worker was accepted';
  exception when others then
    v_msg := sqlerrm;
    if v_msg like 'TEST FAIL%' then raise; end if;
    if v_msg not like '%recovery must be performed by opencode-claimant%' then
      raise exception 'TEST FAIL S5: wrong refusal: %', v_msg;
    end if;
  end;
end $$;

-- S6 (oracle: attempts accounting): recovery with attempts
-- exhausted refuses.
do $$
declare v_msg text;
begin
  perform agent.test_seed('-10 minutes', 4, 4);
  begin
    perform agent.bind_repo_task_lane_session(
      1, 'opencode-claimant', 'ses_old',
      'opencode-claimant-recovery:gone-session');
    raise exception 'TEST FAIL S6: recovery with attempts exhausted was accepted';
  exception when others then
    v_msg := sqlerrm;
    if v_msg like 'TEST FAIL%' then raise; end if;
    if v_msg not like '%recovery attempts exhausted%' then
      raise exception 'TEST FAIL S6: wrong refusal: %', v_msg;
    end if;
  end;
end $$;

-- S7 (oracle: migration-0024 normal-bind contract, unchanged): a
-- normal bind with a claimed task and a live lease binds both
-- stores and emits lane_session_bound.
do $$
begin
  perform agent.test_seed('+10 minutes', 1, 4);
  update agent.repo_task_lane_ledger set session_id = null where task_id = 1;
  update agent.tasks set progress = '{"lane": "opencode"}'::jsonb where id = 1;
  perform agent.bind_repo_task_lane_session(1, 'opencode-claimant', 'ses_new', 'opencode-claimant');
  if (select session_id from agent.repo_task_lane_ledger where task_id = 1) <> 'ses_new'
     or (select progress ->> 'session_id' from agent.tasks where id = 1) <> 'ses_new' then
    raise exception 'TEST FAIL S7: normal bind did not bind both stores';
  end if;
  if not exists (select 1 from agent.events where task_id = 1 and kind = 'lane_session_bound') then
    raise exception 'TEST FAIL S7: lane_session_bound event missing';
  end if;
end $$;

-- S8 (oracle: migration-0024 live-lease guard): a normal bind after
-- lease expiry still refuses.
do $$
declare v_msg text;
begin
  perform agent.test_seed('-10 minutes', 1, 4);
  update agent.repo_task_lane_ledger set session_id = null where task_id = 1;
  update agent.tasks set progress = '{"lane": "opencode"}'::jsonb where id = 1;
  begin
    perform agent.bind_repo_task_lane_session(1, 'opencode-claimant', 'ses_new', 'opencode-claimant');
    raise exception 'TEST FAIL S8: normal bind over an expired lease was accepted';
  exception when others then
    v_msg := sqlerrm;
    if v_msg like 'TEST FAIL%' then raise; end if;
    if v_msg not like '%not claimed with a live lease%' then
      raise exception 'TEST FAIL S8: wrong refusal: %', v_msg;
    end if;
  end;
end $$;

\echo 'migration-0026 tests: all 8 scenarios passed'
