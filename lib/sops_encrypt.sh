#!/usr/bin/env bash

sops_encrypt() {
    if [ $# -ne 1 ]; then
        echo "Usage: <command> | sops_encrypt <output_file>"
        return 1
    fi

    local output_file="$1"

    if ! command -v sops &> /dev/null; then
        echo "Error: SOPS is not installed or not in PATH."
        return 1
    fi

    sops --encrypt <(cat -) > "$output_file"
    echo "Encrypted output saved to $output_file"
}

if [[ "${BASH_SOURCE[0]}" != "${0}" ]]; then
    echo "SOPS encryption function 'sops_encrypt' has been loaded."
    echo "Usage: <command> | sops_encrypt <output_file>"
else
    if [ $# -ne 1 ]; then
        echo "Usage: $0 <output_file>"
        exit 1
    fi
    sops_encrypt "$1"
fi
