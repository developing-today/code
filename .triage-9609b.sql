SELECT count(*) AS payload_bound FROM tasks WHERE status='blocked' AND last_error LIKE '%quarantined by claim path%' AND last_error LIKE '%payload.max_attempts%';
SELECT count(*) AS packet_bound FROM tasks WHERE status='blocked' AND last_error LIKE '%quarantined by claim path%' AND last_error LIKE '%packet.max_attempts%';
SELECT last_error, count(*) AS n FROM tasks WHERE status='blocked' AND last_error LIKE '%quarantined by claim path%' AND last_error NOT LIKE '%max_attempts%' GROUP BY last_error ORDER BY n DESC;
SELECT count(*) AS blocked_1000_quarantined FROM tasks WHERE status='blocked' AND max_attempts=1000 AND last_error LIKE '%quarantined by claim path%';
SELECT count(*) AS blocked_1000_other FROM tasks WHERE status='blocked' AND max_attempts=1000 AND (last_error IS NULL OR last_error NOT LIKE '%quarantined by claim path%');
SELECT id, queue, status, max_attempts, lane_id, substr(last_error,1,100) FROM tasks WHERE max_attempts=1000 AND status NOT IN ('done','cancelled','failed','blocked') ORDER BY id;
SELECT id, queue, status, attempts, max_attempts FROM tasks WHERE id=12126;
SELECT count(*) AS total_rows_1000 FROM tasks WHERE max_attempts=1000;
