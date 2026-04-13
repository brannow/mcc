#!/bin/sh
set -e

TEMP_DIR="${MCC_TEMP_DIR:-/tmp/encoding}"
mkdir -p "$TEMP_DIR"

# If custom temp dir requested, patch the config (copy first — never modify a bind-mount)
if [ "$MCC_TEMP_DIR" ]; then
    sed "s|^temp_dir:.*|temp_dir: $TEMP_DIR|" /etc/mcc/encoding.yaml > /tmp/mcc-config.yaml
    exec mcc --config /tmp/mcc-config.yaml "$@"
fi

exec mcc --config /etc/mcc/encoding.yaml "$@"
