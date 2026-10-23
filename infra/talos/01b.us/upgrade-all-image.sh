set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail

talosctl upgrade --image factory.talos.dev/installer/21c55fcf357cbda853e9f91a1fa5fedf72701b1d7e8bcefe24e9cb60b58892d9:v1.9.1 --nodes 10.10.8.188 # a1 # controlplane
# talosctl upgrade --image factory.talos.dev/installer/21c55fcf357cbda853e9f91a1fa5fedf72701b1d7e8bcefe24e9cb60b58892d9:v1.9.1 --nodes 10.10.12.69 # b1 # controlplane
# talosctl upgrade --image factory.talos.dev/installer/21c55fcf357cbda853e9f91a1fa5fedf72701b1d7e8bcefe24e9cb60b58892d9:v1.9.1 --nodes 10.10.24.137 # c1 # controlplane
talosctl upgrade --image factory.talos.dev/installer/21c55fcf357cbda853e9f91a1fa5fedf72701b1d7e8bcefe24e9cb60b58892d9:v1.9.1 --nodes 10.10.18.43,10.10.21.108,10.10.18.43,10.10.31.114,10.10.16.105,10.10.24.60,10.10.4.141,10.10.22.204
talosctl upgrade --image factory.talos.dev/installer/21c55fcf357cbda853e9f91a1fa5fedf72701b1d7e8bcefe24e9cb60b58892d9:v1.9.1 --nodes 10.10.15.10,10.10.25.241,10.10.8.199,10.10.3.175,10.10.27.240,10.10.30.37,10.10.9.213,10.10.31.127
# talosctl upgrade --image factory.talos.dev/installer/21c55fcf357cbda853e9f91a1fa5fedf72701b1d7e8bcefe24e9cb60b58892d9:v1.9.1 --nodes 10.10.27.240
