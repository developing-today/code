#!/usr/bin/env bash
SYSTEM_NODE=$(which node)
if [ -z "$SYSTEM_NODE" ]; then
    echo "Error: Node.js not found on the system. Please install Node.js first."
    exit 1
else
    echo "System Node.js binary found: $SYSTEM_NODE"
fi
ZED_NODE_DIRS=$(find ~/.local/share/zed/node -name 'node-v*-linux-x64' -type d)
if [ -z "$ZED_NODE_DIRS" ]; then
    echo "Error: No Zed Node.js directories found."
    exit 1
else
    echo "Found $(echo "$ZED_NODE_DIRS" | wc -l) Zed Node.js directories."
    echo "Zed Node.js directories found:"
    echo "$ZED_NODE_DIRS"
fi
for dir in $ZED_NODE_DIRS; do
    NODE_BIN_DIR="$dir/bin"
    NODE_BIN="$NODE_BIN_DIR/node"
    if [ -d "$NODE_BIN_DIR" ]; then
        echo "Node.js binary directory found: $NODE_BIN_DIR"
        if [ -z "$NODE_BIN" ]; then
            echo "Error: Node.js binary not found in $dir."
        elif [ -f "$NODE_BIN" ] || [ -L "$NODE_BIN" ]; then
            echo "Removing existing Node.js binary: $NODE_BIN"
            rm "$NODE_BIN"
        fi
        echo "Creating symlink in $NODE_BIN"
        ln -s "$SYSTEM_NODE" "$NODE_BIN"
    else
        echo "Error: Node.js binary directory not found in $dir."
        exit 1
    fi
done
echo "All Zed Node.js binaries have been symlinked to the system Node.js."
