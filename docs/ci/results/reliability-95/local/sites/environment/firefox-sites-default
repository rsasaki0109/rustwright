#!/bin/sh
# This cloud container denies the user namespaces used by Firefox's content sandbox.
export MOZ_DISABLE_CONTENT_SANDBOX=1
exec /workspace/.rustwright-env/firefox/firefox "$@"
