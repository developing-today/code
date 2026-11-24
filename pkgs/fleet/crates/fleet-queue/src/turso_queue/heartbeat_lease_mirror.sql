CREATE TRIGGER IF NOT EXISTS heartbeat_lease_mirror
AFTER UPDATE OF lease_until ON tasks
WHEN OLD.status = 'claimed' AND NEW.status = 'claimed'
 AND NEW.worker_id IS NOT NULL AND NEW.worker_id IS OLD.worker_id
 AND NEW.claimed_at IS OLD.claimed_at AND NEW.lane_id IS OLD.lane_id
 AND NEW.lease_until IS NOT NULL
BEGIN
    INSERT INTO leases (task_id, worker_id, lease_until, heartbeat_at)
    VALUES (NEW.id, NEW.worker_id, NEW.lease_until, NEW.updated_at)
    ON CONFLICT(task_id) DO UPDATE SET worker_id = excluded.worker_id,
        lease_until = excluded.lease_until, heartbeat_at = excluded.heartbeat_at;
END
