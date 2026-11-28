-- #10671: one compare-and-swap against the exact selected row and event.
-- Parameters: next status, next error, now, task id, raw snapshot, last event id.
-- Refuse conflicting leases rather than deleting one after making a row claimable.
UPDATE tasks SET status = ?1, last_error = ?2, updated_at = ?3
WHERE id = ?4 AND status = 'blocked'
  AND {snapshot} = ?5
  AND worker_id IS NULL AND lease_until IS NULL
  AND NOT EXISTS (SELECT 1 FROM leases WHERE task_id = tasks.id)
  AND (SELECT MAX(id) FROM events WHERE task_id = tasks.id) = ?6
  AND EXISTS (
    SELECT 1 FROM events
    WHERE id = ?6 AND task_id = tasks.id AND kind = 'blocked'
      AND CASE WHEN json_valid(detail) THEN json_extract(detail, '$.via') END
        IN ('claim_payload_quarantine', 'quarantine_recheck')
  )
