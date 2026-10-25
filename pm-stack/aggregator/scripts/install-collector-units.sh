#!/usr/bin/env bash
# install-collector-units.sh — idempotent installer for the sandbox-side
# fleet-feed collector systemd units (fleet-feed-collector.{service,timer}).
#
# /etc/systemd/system does not survive sandbox recycles, so ~/tools/init.sh
# calls this on every boot where the collector timer is not active (the
# same restore pattern as the fleet-units and airthings installers).
# Source of truth for the units is this package's systemd/ directory, as
# checked out in the feed clone (~/git/pm-feed). Safe to re-run any time.
set -eu
DIR="$(cd "$(dirname "$0")/.." && pwd)" # the aggregator package dir

cp "$DIR/systemd/fleet-feed-collector.service" /etc/systemd/system/fleet-feed-collector.service
cp "$DIR/systemd/fleet-feed-collector.timer" /etc/systemd/system/fleet-feed-collector.timer
systemctl daemon-reload
systemctl enable --now fleet-feed-collector.timer
systemctl is-active --quiet fleet-feed-collector.timer &&
  echo "fleet-feed-collector.timer active" ||
  {
    echo "FAILED to activate fleet-feed-collector.timer"
    exit 1
  }
