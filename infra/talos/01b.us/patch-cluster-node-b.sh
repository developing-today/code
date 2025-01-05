export TALOSCONFIG=secrets/talosconfig

talosctl apply-config --insecure --file "./secrets/controlplane.yaml" --nodes 10.10.12.69 # b1

talosctl config endpoint 10.10.0.42 10.10.8.188 10.10.12.69 10.10.24.137

talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.25.241 # b2
talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.8.199 # b3
talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.3.175 # b4
talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.27.240 # b5
talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.30.37 # b6
talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.9.213 # b7
talosctl apply-config --insecure --file "./secrets/worker.yaml" --nodes 10.10.31.127 # b8

talosctl config node \
10.10.0.42 \
10.10.8.188 \
10.10.12.69 \
10.10.18.178 \
10.10.18.43 \
10.10.21.108 \
10.10.14.112 \
10.10.24.164 \
10.10.8.0 \
10.10.20.128 \
10.10.18.43 \
10.10.31.114 \
10.10.16.105 \
10.10.24.136 \
10.10.24.60 \
10.10.4.141 \
10.10.22.204 \
10.10.15.10 \
\
10.10.25.241 \
10.10.8.199 \
10.10.3.175 \
10.10.27.240 \
10.10.30.37 \
10.10.9.213 \
10.10.31.127 \
#

talosctl etcd status -n 10.10.8.188,10.10.12.69,10.10.24.137
