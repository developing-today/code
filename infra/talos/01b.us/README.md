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

Talos Linux Image Factory

The Talos Linux Image Factory, developed by Sidero Labs, Inc., offers a method to download various boot assets for Talos Linux.

For more information about the Image Factory API and the available image formats, please visit the GitHub repository.

Version: v0.6.4
Loading...
Schematic Ready
Your image schematic ID is: 21c55fcf357cbda853e9f91a1fa5fedf72701b1d7e8bcefe24e9cb60b58892d9
customization:
    systemExtensions:
        officialExtensions:
            - siderolabs/drbd
            - siderolabs/tailscale
            - siderolabs/xen-guest-agent
First Boot

Here are the options for the initial boot of Talos Linux on a bare-metal machine or a generic virtual machine:
https://factory.talos.dev/image/21c55fcf357cbda853e9f91a1fa5fedf72701b1d7e8bcefe24e9cb60b58892d9/v1.9.0/metal-amd64.iso
ISO
    https://factory.talos.dev/image/21c55fcf357cbda853e9f91a1fa5fedf72701b1d7e8bcefe24e9cb60b58892d9/v1.9.0/metal-amd64.iso (ISO documentation) 
Disk Image (raw)
    https://factory.talos.dev/image/21c55fcf357cbda853e9f91a1fa5fedf72701b1d7e8bcefe24e9cb60b58892d9/v1.9.0/metal-amd64.raw.zst 
Disk Image (qcow2)
    https://factory.talos.dev/image/21c55fcf357cbda853e9f91a1fa5fedf72701b1d7e8bcefe24e9cb60b58892d9/v1.9.0/metal-amd64.qcow2 
PXE boot (iPXE script)
    https://pxe.factory.talos.dev/pxe/21c55fcf357cbda853e9f91a1fa5fedf72701b1d7e8bcefe24e9cb60b58892d9/v1.9.0/metal-amd64 (PXE documentation)

Initial Installation

For the initial installation of Talos Linux (not applicable for disk image boot), add the following installer image to the machine configuration:
factory.talos.dev/installer/21c55fcf357cbda853e9f91a1fa5fedf72701b1d7e8bcefe24e9cb60b58892d9:v1.9.0
Upgrading Talos Linux

To upgrade Talos Linux on the machine, use the following image:
factory.talos.dev/installer/21c55fcf357cbda853e9f91a1fa5fedf72701b1d7e8bcefe24e9cb60b58892d9:v1.9.0
Documentation

    What's New in Talos v1.9
    Support Matrix for v1.9
    Getting Started Guide
    Bare-metal Network Configuration
    Production Cluster Guide
    Troubleshooting Guide

Extra Assets

Kernel Image
    https://factory.talos.dev/image/21c55fcf357cbda853e9f91a1fa5fedf72701b1d7e8bcefe24e9cb60b58892d9/v1.9.0/kernel-amd64 
Kernel Command Line
    https://factory.talos.dev/image/21c55fcf357cbda853e9f91a1fa5fedf72701b1d7e8bcefe24e9cb60b58892d9/v1.9.0/cmdline-metal-amd64 
Initramfs Image
    https://factory.talos.dev/image/21c55fcf357cbda853e9f91a1fa5fedf72701b1d7e8bcefe24e9cb60b58892d9/v1.9.0/initramfs-amd64.xz
