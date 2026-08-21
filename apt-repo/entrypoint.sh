#!/bin/sh
set -eu

# Dokploy mounts the named volume at /repo. Seed it exactly once so deploys never
# overwrite packages or repository metadata that were published after the first run.
if [ ! -e /repo/.repository-initialized ]; then
  mkdir -p /repo
  cp -a /seed/. /repo/
  touch /repo/.repository-initialized
fi
