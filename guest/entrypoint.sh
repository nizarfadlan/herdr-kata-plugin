#!/bin/sh
# Guest entrypoint. Starts as root so it can apply the egress firewall, then
# drops to the unprivileged `agent` user before the agent itself starts.
# The container stays confined by the microVM boundary either way.
set -eu

/usr/local/bin/egress.sh

exec setpriv --reuid=agent --regid=agent --clear-groups -- "$@"
