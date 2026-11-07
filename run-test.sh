#!/bin/bash
cd ~/git/fleet/6266/pkgs/id
CARGO_BUILD_JOBS=4 cargo test --lib handoff_ > ~/git/fleet/6266/test-6266-lib.log 2>&1
