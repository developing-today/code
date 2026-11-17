SELECT count(*) AS quarantined FROM tasks WHERE status='blocked' AND last_error LIKE '%quarantined by claim path%';
SELECT queue, count(*) AS n FROM tasks WHERE status='blocked' AND last_error LIKE '%quarantined by claim path%' GROUP BY queue ORDER BY n DESC;
SELECT CASE WHEN last_error LIKE '%packet.max_attempts%' THEN 'max_attempts_bound' WHEN last_error LIKE '%payload.max_attempts%' THEN 'payload_max_attempts_bound' WHEN last_error LIKE '%payload.schema%' THEN 'payload.schema' WHEN last_error LIKE '%payload.problem%' THEN 'payload.problem' WHEN last_error LIKE '%payload.kind%' THEN 'payload.kind' ELSE 'other' END AS shape, count(*) AS n FROM tasks WHERE status='blocked' AND last_error LIKE '%quarantined by claim path%' GROUP BY shape ORDER BY n DESC;
SELECT max_attempts, count(*) AS n FROM tasks WHERE status='blocked' AND last_error LIKE '%quarantined by claim path%' GROUP BY max_attempts ORDER BY n DESC;
SELECT id, queue, status, max_attempts, substr(last_error,1,120) AS err FROM tasks WHERE id IN (16147,15155,16120,12126) ORDER BY id;
SELECT count(*) AS nonterminal_1000 FROM tasks WHERE status NOT IN ('done','cancelled','failed','blocked') AND max_attempts=1000;
SELECT status, count(*) AS n FROM tasks WHERE max_attempts=1000 GROUP BY status ORDER BY n DESC;
