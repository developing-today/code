- uses https://factory.talos.dev/?arch=amd64&cmdline-set=true&extensions=-&extensions=siderolabs%2Fdrbd&extensions=siderolabs%2Ftailscale&extensions=siderolabs%2Fxen-guest-agent&platform=metal&target=metal&version=1.9.0
  - Your image schematic ID is: 21c55fcf357cbda853e9f91a1fa5fedf72701b1d7e8bcefe24e9cb60b58892d9 
```
customization:
    systemExtensions:
        officialExtensions:
            - siderolabs/drbd
            - siderolabs/tailscale
            - siderolabs/xen-guest-agent
```
