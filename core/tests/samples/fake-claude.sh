#!/bin/sh
exec "$(dirname "$0")/fake-agent.sh" claude "$@"
