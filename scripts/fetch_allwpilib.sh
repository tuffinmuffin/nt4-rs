#!/bin/sh
# Fetch the parts of allwpilib the audits read, at one commit or tag.
#
# Usage: scripts/fetch_allwpilib.sh <dir> <commit or tag>
#
# The ntcore and wpiutil tests (scripts/upstream_audit.py) and the struct
# definitions (scripts/schema_audit.py), as a shallow sparse checkout.
#
# Written by Claude.
set -eu
dir=$1
ref=$2
git init -q "$dir"
cd "$dir"
git remote add origin https://github.com/wpilibsuite/allwpilib.git
git sparse-checkout set --no-cone \
  /ntcore/src/test/ \
  /wpiutil/src/test/native/cpp/struct/ \
  '/wpimath/src/main/native/include/wpi/math/*/struct/' \
  '/wpimath/src/main/java/org/wpilib/math/*/struct/' \
  /hal/src/main/native/include/wpi/hal/DriverStationTypes.hpp \
  /hal/src/main/java/org/wpilib/hardware/hal/struct/
git fetch -q --depth 1 --filter=blob:none origin "$ref"
git checkout -q FETCH_HEAD
