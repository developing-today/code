set -Eexuo pipefail # https://vaneyckt.io/posts/safer_bash_scripts_with_set_euxo_pipefail

talosctl apply-config --file "./secrets/controlplane.yaml" --nodes 10.10.8.188  # a1
talosctl apply-config --file "./secrets/controlplane.yaml" --nodes 10.10.12.69  # b1
talosctl apply-config --file "./secrets/controlplane.yaml" --nodes 10.10.24.137 # c1
talosctl apply-config --file "./secrets/worker.yaml" --nodes 10.10.18.43
talosctl apply-config --file "./secrets/worker.yaml" --nodes 10.10.21.108
talosctl apply-config --file "./secrets/worker.yaml" --nodes 10.10.31.114 # a2
talosctl apply-config --file "./secrets/worker.yaml" --nodes 10.10.16.105 # a5
talosctl apply-config --file "./secrets/worker.yaml" --nodes 10.10.24.60  # c2
talosctl apply-config --file "./secrets/worker.yaml" --nodes 10.10.4.141  # c3
talosctl apply-config --file "./secrets/worker.yaml" --nodes 10.10.22.204 # c4
talosctl apply-config --file "./secrets/worker.yaml" --nodes 10.10.15.10  # c5
talosctl apply-config --file "./secrets/worker.yaml" --nodes 10.10.25.241 # b2
talosctl apply-config --file "./secrets/worker.yaml" --nodes 10.10.8.199  # b3
talosctl apply-config --file "./secrets/worker.yaml" --nodes 10.10.3.175  # b4
talosctl apply-config --file "./secrets/worker.yaml" --nodes 10.10.27.240 # b5
talosctl apply-config --file "./secrets/worker.yaml" --nodes 10.10.30.37  # b6
talosctl apply-config --file "./secrets/worker.yaml" --nodes 10.10.9.213  # b7
talosctl apply-config --file "./secrets/worker.yaml" --nodes 10.10.31.127 # b8
