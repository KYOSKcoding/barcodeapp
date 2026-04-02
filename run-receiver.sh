#!/bin/bash
set -e

cd "$(dirname "$0")"
cargo build -p receiver

exec env -i \
  HOME=$HOME \
  PATH=$PATH \
  DISPLAY=$DISPLAY \
  WAYLAND_DISPLAY=$WAYLAND_DISPLAY \
  XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR \
  DBUS_SESSION_BUS_ADDRESS=$DBUS_SESSION_BUS_ADDRESS \
  XAUTHORITY=$XAUTHORITY \
  XDG_DATA_DIRS=$XDG_DATA_DIRS \
  GSETTINGS_SCHEMA_DIR="$PWD/schemas" \
  target/debug/receiver
