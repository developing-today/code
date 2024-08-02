#!/usr/bin/env bash

# Function to display usage information
usage() {
    echo "Usage: $0 <source_directory> <target_directory>"
    echo "Moves files from source_directory to target_directory, handling duplicates by appending _1, _2, etc."
    exit 1
}

# Check if correct number of arguments is provided
if [ "$#" -ne 2 ]; then
    usage
fi

source_dir="$1"
target_dir="$2"

# Check if source directory exists
if [ ! -d "$source_dir" ]; then
    echo "Error: Source directory '$source_dir' does not exist."
    exit 1
fi

# Ensure target directory exists
mkdir -p "$target_dir"

# Loop through all files in the source directory
for file in "$source_dir"/*; do
    # Skip if it's not a file
    [ -f "$file" ] || continue

    # Get just the filename
    filename=$(basename "$file")
    
    # If the file doesn't exist in the target directory, just move it
    if [ ! -e "$target_dir/$filename" ]; then
        mv "$file" "$target_dir/"
    else
        # File exists, so we need to rename it
        counter=1
        while [ -e "$target_dir/${filename%.*}_$counter.${filename##*.}" ]; do
            ((counter++))
        done
        mv "$file" "$target_dir/${filename%.*}_$counter.${filename##*.}"
    fi
done

echo "File moving complete. Check $target_dir for results."

