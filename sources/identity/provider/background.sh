#!/usr/bin/env bash

cd "${0%/*}"

[[ -f 'config.sh' ]] && source config.sh

execute_file() {
    local file=$1
    echo "Executing file: $file"
    chmod +x "$BACKGROUND_JOB_EXECUTING_DIR/$file"
    if bash "$BACKGROUND_JOB_EXECUTING_DIR/$file"; then
        echo "File executed successfully: $file"
        mv "$BACKGROUND_JOB_EXECUTING_DIR/$file" "$BACKGROUND_JOB_COMPLETE_DIR/"
    else
        echo "File execution failed: $file"
        mv "$BACKGROUND_JOB_EXECUTING_DIR/$file" "$BACKGROUND_JOB_FAILED_DIR/"
    fi
}
monitor_files() {
    while true; do
        for file in "$BACKGROUND_JOB_DIR"/*; do
            if [ -f "$file" ]; then
                mv "$file" "$BACKGROUND_JOB_EXECUTING_DIR/"
                execute_file "$(basename "$file")" &
            fi
        done
        sleep 1
    done
}
mkdir -p "$BACKGROUND_JOB_EXECUTING_DIR" "$BACKGROUND_JOB_COMPLETE_DIR" "$BACKGROUND_JOB_FAILED_DIR"
mv "$BACKGROUND_JOB_EXECUTING_DIR"/* "$BACKGROUND_JOB_FAILED_DIR/" 2>/dev/null
monitor_files &
