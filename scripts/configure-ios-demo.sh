#!/bin/sh
set -eu

repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
env_file="$repo_dir/.env"
output_file="$repo_dir/ios-demo/Config.local.xcconfig"

if [ ! -f "$env_file" ]; then
    echo "Missing $env_file" >&2
    exit 1
fi

api_key=$(sed -n 's/^ALCHEMY_API_KEY=//p' "$env_file" | tail -n 1)
if [ -z "$api_key" ]; then
    echo "ALCHEMY_API_KEY is missing from $env_file" >&2
    exit 1
fi

umask 077
printf 'ALCHEMY_API_KEY = %s\n' "$api_key" > "$output_file"
echo "Wrote $output_file"
