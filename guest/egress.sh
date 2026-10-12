#!/bin/sh
# Guest egress firewall, applied by the entrypoint before the agent starts.
#
# HERDR_EGRESS_ALLOW is a comma-separated list of host:port pairs supplied by
# the plugin from the profile's egress_allowlist. When it is set, OUTPUT
# defaults to DROP and only the allowlist (plus loopback and established
# connections) is permitted. When it is empty, the firewall is left off.
#
# Needs the NET_ADMIN capability, which the plugin grants only when the
# allowlist is non-empty.
set -eu

allow="${HERDR_EGRESS_ALLOW:-}"
if [ -z "$allow" ]; then
    echo "egress: no allowlist configured, leaving the firewall off" >&2
    exit 0
fi

if ! command -v iptables >/dev/null 2>&1; then
    echo "egress: iptables is not installed, cannot enforce the allowlist" >&2
    exit 1
fi

iptables -F OUTPUT
iptables -P OUTPUT DROP
iptables -A OUTPUT -o lo -j ACCEPT
iptables -A OUTPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT

# Split on commas without forking a subshell per entry.
old_ifs="$IFS"
IFS=','
for entry in $allow; do
    IFS="$old_ifs"
    entry="$(echo "$entry" | tr -d '[:space:]')"
    host="${entry%%:*}"
    port="${entry##*:}"
    if [ -z "$host" ] || [ -z "$port" ] || [ "$host" = "$entry" ]; then
        echo "egress: ignoring malformed entry '$entry'" >&2
        continue
    fi
    # Resolve once, at boot; the rules pin the IPs.
    ips="$(getent hosts "$host" | awk '{print $1}')"
    if [ -z "$ips" ]; then
        echo "egress: cannot resolve '$host', skipping" >&2
        continue
    fi
    for ip in $ips; do
        iptables -A OUTPUT -p tcp -d "$ip" --dport "$port" -j ACCEPT
    done
    IFS=','
done
IFS="$old_ifs"

echo "egress: default-DROP with allowlist: $allow" >&2
