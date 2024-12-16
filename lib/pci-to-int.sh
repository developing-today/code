#!/usr/bin/env bash

while IFS= read -r line; do
    if [[ $line =~ ^([0-9a-f]+):([0-9a-f]+)\.[0-9].*VGA.*$ ]]; then
        bus_id="${BASH_REMATCH[1]}"
        dev_id="${BASH_REMATCH[2]}"

        # Convert hex to decimal
        bus_dec=$((16#$bus_id))
        dev_dec=$((16#$dev_id))

        if [[ $line == *"NVIDIA"* ]]; then
            nvidia="      nvidiaBusId = \"PCI:$bus_dec:$dev_dec:0\";"
        elif [[ $line == *"AMD"* ]]; then
            amd="      amdgpuBusId = \"PCI:$bus_dec:$dev_dec:0\";"
        fi
    fi
done < <(lspci | grep "VGA")

[ ! -z "$amd" ] && echo "$amd"
[ ! -z "$nvidia" ] && echo "$nvidia"
