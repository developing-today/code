#!/bin/bash
git restore pkgs/id/src/world_store.rs
# To actually appease both clippy unused imports and unused variables:
sed -i 's/ JOURNAL_VERSION_EXECUTION_PROFILE,//g' pkgs/id/src/world_store.rs
