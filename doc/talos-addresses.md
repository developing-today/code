- vip
  - https://10.10.0.42:6443
  - https://vip.01b.us:6443
  - static
  - Don’t use the VIP as the endpoint in the talosconfig, as the VIP is bound to etcd and kube-apiserver health, and you will not be able to recover from a failure of either of those components using Talos API.
- talosconfig endpoint
  - none for now, directly routable through talosctl
- controllers
  - a1
    - 10.10.2.185
    - static
  - a2
    - 10.10.30.13
    - static
  - b1
    - 10.10.4.114
    - static
- workers
  - a3
    - 10.10.13.212
  - a4
    - 10.10.9.224
  - b2
    - 10.10.18.238
  - b3
    - 10.10.26.178
  - b4
    - 10.10.4.187
  - b5
    - 10.10.14.89
  - b6
    - 10.10.29.103