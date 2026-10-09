#!/bin/sh
# This cloud container cannot use Chromium's setuid or user-namespace sandbox.
export XDG_CONFIG_HOME=/workspace/.rustwright-env/xdg-config
export XDG_CACHE_HOME=/workspace/.rustwright-env/xdg-cache
exec /usr/bin/chromium --no-sandbox --disable-dev-shm-usage "$@"
