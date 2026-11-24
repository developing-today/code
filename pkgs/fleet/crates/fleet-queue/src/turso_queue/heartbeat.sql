UPDATE tasks
   SET lease_until = CASE WHEN julianday(lease_until) >= julianday(?1)
                          THEN lease_until ELSE ?1 END,
       progress = ?2, updated_at = ?3
 WHERE id = ?4 AND status = 'claimed' AND worker_id = ?5
   AND claimed_at IS ?6 AND lane_id IS ?7 AND attempts = ?8
