#!/usr/bin/env bash
# Installs the system libraries GPUI's Linux backends link against, on Debian
# or Ubuntu. CI runs it before building; run it once on a new Linux machine.
set -euo pipefail

sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  build-essential \
  pkg-config \
  libfontconfig-dev \
  libfreetype-dev \
  libwayland-dev \
  libx11-xcb-dev \
  libxkbcommon-dev \
  libxkbcommon-x11-dev
