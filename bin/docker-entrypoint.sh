#!/bin/sh
# Docker entrypoint: apply pending database migrations before the server
# starts, then hand off to the hq binary. Other commands (e.g.
# `dokku run hq migrate`) run unchanged.
set -e

if [ "$1" = "serve" ]; then
    /hq migrate
fi

exec /hq "$@"
