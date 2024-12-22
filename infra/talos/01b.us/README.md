- uses https://factory.talos.dev/?arch=amd64&cmdline-set=true&extensions=-&extensions=siderolabs%2Fdrbd&extensions=siderolabs%2Ftailscale&extensions=siderolabs%2Fxen-guest-agent&platform=metal&target=metal&version=1.9.0

```
customization:
    systemExtensions:
        officialExtensions:
            - siderolabs/drbd
            - siderolabs/tailscale
            - siderolabs/xen-guest-agent
```
