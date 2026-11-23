# Telemetry

What we log on every boot and where.

## Files

| File | Format | Purpose |
|---|---|---|
| `~/logs/init.log` | Human-readable | Full boot log with per-step details |
| `~/logs/init.jsonl` | Compact JSON (one object per line) | Machine-parseable boot telemetry |
| `~/logs/init-status` | Key=value | Latest boot status (read by init.sh for prev-boot info) |
| `~/logs/.init-boot-id` | Plain text | Marker file (boot ID of last successful init) |

## What We Capture Per Boot

- **Identity**: boot_id, boot_time, previous boot_id and end time (for boot frequency)
- **System**: kernel version, OS version, CPU count, memory, disk free, nix-persist size
- **Network**: whether `cache.nixos.org` is reachable (TCP/HTTPS, since ICMP is blocked), 3-sample latency average
- **Nix health**: nix version, store path count, profile binary count
- **Timing**: per-step duration, total init seconds
- **Result**: SUCCESS or FAILURES=N, quota info

## Example JSONL Entry

```json
{"ts":"2026-09-30 01:08:11 CDT","boot_id":"c9b282a6-...","boot_time":"2026-09-29 19:00:41","prev_boot_id":"c9b282a6-...","result":"SUCCESS","init_secs":3,"kernel":"7.0.0-39-generic","os":"Ubuntu 24.04.5 LTS","cpus":"2","mem":"7935M","disk_home":"96G free of 100G","nix_persist":"1.4G","net_ok":"yes","net_lat_s":0.243,"nix_version":"nix (Nix) 2.34.8","store_paths":2880,"profile_bins":14}
```

## Network Baseline

TCP/HTTPS to `cache.nixos.org` (20 samples, ICMP blocked):
- min 0.159s, avg 0.351s, max 2.868s (outlier), stddev 0.580s, 0 failures

## Boot History

Six boots observed (from `init.log`):
- 2026-09-29 06:08 UTC — `1bb584e0`
- 2026-09-29 13:06 UTC — `31504870`
- 2026-09-29 15:53 UTC — `757b1262`
- 2026-09-29 18:16 UTC — `a17ec461`
- 2026-09-29 20:32 UTC — `8af28114`
- 2026-09-30 00:00 UTC — `c9b282a6` (current)
