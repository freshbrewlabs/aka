#!/bin/bash
set -euo pipefail

# Script to build all services in aka

# Color codes for output
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo "running all build files..."
echo ""

# Find all build.sh files and execute them. One stack failing must not stop the
# others: under `set -e` a failing subshell aborts the loop before the ✗ line
# ever prints, hence the `|| exit_code=$?`.
failures=""

while IFS= read -r -d '' file; do
    echo -e "${YELLOW}================================================================================${NC}"
    echo -e "${GREEN}Executing: $file${NC}"

    # Get the directory containing the file
    file_dir=$(dirname "$file")

    # Make the file executable
    chmod +x "$file"

    # Execute the file from its directory
    exit_code=0
    (cd "$file_dir" && bash "./$(basename "$file")") || exit_code=$?

    if [ "$exit_code" -eq 0 ]; then
        echo -e "${GREEN}✓ Successfully executed: $file${NC}"
    else
        echo -e "${RED}✗ Failed to execute: $file (exit code: $exit_code)${NC}"
        failures="$failures $file"
    fi
    echo ""
    echo -e "${YELLOW}================================================================================${NC}"
done < <(find ./src -type f -name "build.sh" -print0)

if [ -n "$failures" ]; then
    echo -e "${RED}build.sh: failed:$failures${NC}" >&2
    exit 1
fi

echo ""
echo -e "${GREEN}All build.sh files have been processed.${NC}"
