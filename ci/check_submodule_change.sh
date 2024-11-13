#!/bin/bash

# Ensure a submodule path is provided
if [ -z "$1" ]; then
    echo "Usage: $0 <submodule_path>"
    exit 1
fi

# Submodule path
SUBMODULE_PATH="$1"

# Fetch the latest commit from the main branch
echo "Fetching latest commit from origin/main..."
git fetch origin main

# Get the SHA of the submodule on the main branch
BASE_SHA=$(git ls-tree origin/main @ "$SUBMODULE_PATH" | awk '{ print $3 }')
# Get the SHA of the submodule in the current HEAD
CURRENT_SHA=$(git ls-tree HEAD @ "$SUBMODULE_PATH" | awk '{ print $3 }')

echo "Base SHA on main branch: $BASE_SHA"
echo "Current SHA: $CURRENT_SHA"

# Check if the submodule SHA has changed
if [ "$BASE_SHA" != "$CURRENT_SHA" ]; then
    echo "Submodule SHA has changed"
    echo "build=true" >> "$GITHUB_OUTPUT"
else
    echo "Submodule SHA has not changed"
    echo "build=false" >> "$GITHUB_OUTPUT"
fi
