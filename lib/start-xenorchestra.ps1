#!/usr/bin/env pwsh

New-Item -ItemType Directory -Force -Path "$PSScriptRoot\data\xo-server"
New-Item -ItemType Directory -Force -Path "$PSScriptRoot\data\redis"

#podman run -itd `
docker run -itd `
  --stop-timeout 60 `
  --restart unless-stopped `
  --cap-add sys_admin `
  --cap-add dac_read_search `
  --security-opt apparmor:unconfined `
  -p 1337:80 `
  ronivay/xen-orchestra
  # -v "$PSScriptRoot/data/xo-server:/var/lib/xo-server" `
  # -v "$PSScriptRoot/data/redis:/var/lib/redis" `
