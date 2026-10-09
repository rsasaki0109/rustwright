#!/bin/bash
set -euo pipefail
cd /workspace/rustwright
. /workspace/.rustwright-env/activate.sh
export CARGO_INCREMENTAL=0
git config --global --add safe.directory /workspace/rustwright
id
/usr/bin/python3 --version
printf 'init: '
cat /proc/1/comm
# This is a disposable container's browser certificate store. TLS remains enabled.
mkdir -p /root/.pki/nssdb
/workspace/.rustwright-env/site-nss/root/usr/bin/certutil -N --empty-password -d sql:/root/.pki/nssdb
/workspace/.rustwright-env/site-nss/root/usr/bin/certutil -A -d sql:/root/.pki/nssdb -n rustwright-environment-proxy-ca -t C,, -i /usr/local/share/ca-certificates/environment-proxy-ca.crt
/usr/bin/python3 target/reliability95/container/runner.py native
# Site-operation failures are retained as observations; they must not erase native results.
set +e
/usr/bin/python3 target/reliability95/container/runner.py sites --mode headed
site_exit=$?
printf 'headed site observer exit: %s\n' "$site_exit"
exit "$site_exit"
