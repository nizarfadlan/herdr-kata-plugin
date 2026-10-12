#!/bin/sh
# Adds key bindings for the kata.microvm plugin to the Herdr config and reloads it.
# Usage: scripts/install-keybindings.sh
#
# The config file is resolved the way Herdr resolves it: HERDR_CONFIG_PATH,
# then $XDG_CONFIG_HOME/herdr/config.toml, then ~/.config/herdr/config.toml.
# A binding is appended only when its action is not bound yet and its chord is
# free, so the script can run any number of times. A binding that sits on a
# chord Herdr 0.9 uses itself is reported, not changed. The file is validated
# with "herdr config check" afterwards and restored from a backup when Herdr
# rejects it. HERDR_BIN_PATH names the herdr binary (Herdr sets it for plugin
# actions); otherwise herdr on PATH.
#
# Report lines on stdout, one per event, read by the install-keybindings action:
#   config: PATH
#   bound KEY -> kata.microvm.ACTION
#   already bound: kata.microvm.ACTION (KEY)
#   warning: TEXT
#   reloaded
# Everything Herdr itself prints goes to stderr so it cannot be mistaken for a report line.
set -eu

if [ -n "${HERDR_CONFIG_PATH:-}" ]; then
  config=$HERDR_CONFIG_PATH
elif [ -n "${XDG_CONFIG_HOME:-}" ]; then
  config=$XDG_CONFIG_HOME/herdr/config.toml
elif [ -n "${HOME:-}" ]; then
  config=$HOME/.config/herdr/config.toml
else
  printf 'warning: neither HERDR_CONFIG_PATH, XDG_CONFIG_HOME nor HOME is set; cannot find the Herdr config\n'
  exit 1
fi
case "$config" in
  */*) config_dir=${config%/*} ;;
  *) config_dir=. ;;
esac
[ -n "$config_dir" ] || config_dir=/
[ -d "$config_dir" ] || mkdir -p "$config_dir"
[ -f "$config" ] || : > "$config"
printf 'config: %s\n' "$config"

herdr=${HERDR_BIN_PATH:-herdr}
plugin="kata.microvm"

# chord -> action, mirroring the nono plugin's chords.
bindings="prefix+Shift+a:start-agent
prefix+Shift+o:microvms
prefix+Shift+b:reconnect
prefix+Shift+s:open-shell"

backup="$config.bak.herdr-kata"
cp "$config" "$backup"

added=0
printf '%s\n' "$bindings" | while IFS=: read -r chord action; do
  [ -n "$chord" ] || continue
  full="$plugin.$action"
  if grep -qF "$full" "$config" 2>/dev/null; then
    printf 'already bound: %s\n' "$full"
    continue
  fi
  if grep -qF "\"$chord\"" "$config" 2>/dev/null; then
    printf 'warning: chord %s is already used; not binding %s\n' "$chord" "$full"
    continue
  fi
  {
    printf '\n[[keys.command]]\n'
    printf 'key = "%s"\n' "$chord"
    printf 'type = "plugin_action"\n'
    printf 'command = "%s"\n' "$full"
    printf 'description = "%s (kata.microvm)"\n' "$action"
  } >> "$config"
  printf 'bound %s -> %s\n' "$chord" "$full"
  added=$((added + 1))
done

if command -v "$herdr" >/dev/null 2>&1 && [ "$added" -gt 0 ]; then
  if "$herdr" config check 1>&2; then
    if "$herdr" config reload 1>&2; then
      printf 'reloaded\n'
    else
      printf 'warning: could not reload the Herdr config\n'
    fi
  else
    cp "$backup" "$config"
    printf 'warning: Herdr rejected the config; restored the backup\n'
    exit 1
  fi
elif [ "$added" -eq 0 ]; then
  printf 'warning: nothing new to bind\n'
else
  printf 'warning: herdr binary not found; config updated but not reloaded\n'
fi
