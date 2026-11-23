# Systemd Services

## What Works

**Systemd is fully functional.** PID 1 is systemd, `systemctl` works, and services can be installed, started, stopped, and enabled.

Verified:
- Created a test service in `/etc/systemd/system/`
- `systemctl daemon-reload`, `start`, `is-active` all work
- Cleaned up after testing

## The Catch

Systemd unit files live in `/etc/systemd/system/`, which is on the **ephemeral overlay**. They are wiped on every recycle.

### Pattern: Boot-Restored Services

To run a persistent service:

1. Store the unit file source in `~/tools/` (persistent)
2. Add a `run_step` to `~/tools/init.sh` that:
   - Copies the unit file to `/etc/systemd/system/`
   - Runs `systemctl daemon-reload`
   - Runs `systemctl enable --now <service>`

Example unit file:
```ini
[Unit]
Description=My persistent service
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/my-program
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
```

## Alternative: Detached Processes

For simpler cases, a background process with PID-file guarding works:

```bash
# Launch (survives exec termination)
nohup my-daemon > ~/logs/my-daemon.log 2>&1 &
echo $! > ~/logs/my-daemon.pid
disown

# Check before launching another
if [ -f ~/logs/my-daemon.pid ] && kill -0 $(cat ~/logs/my-daemon.pid) 2>/dev/null; then
  echo "already running"
else
  # launch it
fi
```

Verified: a `nohup`'d process (PID 33011) survived exec termination and was visible in a new exec.

### Trade-offs

| | systemd | Detached process |
|---|---|---|
| Auto-restart on crash | Yes (`Restart=always`) | No (manual) |
| Boot integration | Via init.sh | Via init.sh or cron |
| Complexity | Higher (unit file) | Lower (shell) |
| Logging | journald | Manual redirect |

## Limitations

- No `CAP_SYS_ADMIN` for exotic mounts
- `mknod` denied (no `/dev/fuse`, no FUSE filesystems)
- ICMP blocked (use TCP for network health checks)
