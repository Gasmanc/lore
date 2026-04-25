#!/bin/bash
# Run by the `dev.lore.check-updates` launchd agent.
#
# Invokes `lore check-updates`. If any installed package is out of date
# (exit code 1), posts a macOS notification via osascript.
#
# Output is logged to ~/Library/Logs/lore-check-updates.log so you can see
# what happened on the last run even if you missed the notification.

set -u

LORE_BIN="${LORE_BIN:-/opt/homebrew/bin/lore}"
LOG="${HOME}/Library/Logs/lore-check-updates.log"

mkdir -p "$(dirname "$LOG")"

{
  echo "── $(date '+%Y-%m-%d %H:%M:%S') ──"
  if [ ! -x "$LORE_BIN" ]; then
    echo "lore binary not found at $LORE_BIN"
    exit 0
  fi

  output=$("$LORE_BIN" check-updates 2>&1)
  status=$?
  echo "$output"

  if [ "$status" -ne 0 ]; then
    # Strip ANSI colour codes so the notification body is readable.
    clean=$(echo "$output" | sed $'s/\x1b\\[[0-9;]*m//g')
    stale=$(echo "$clean" | grep -c 'UPDATE' || true)
    osascript -e "display notification \"${stale} package(s) out of date — run 'lore check-updates' for details.\" with title \"lore: docs need refreshing\""
  fi
} >> "$LOG" 2>&1
