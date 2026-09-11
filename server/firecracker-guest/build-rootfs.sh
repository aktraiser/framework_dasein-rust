#!/usr/bin/env bash
# Run on Linux. Export an operator-selected Python image, then build ext4 offline.
set -euo pipefail
if [[ $# -ne 2 ]]; then echo "Usage: $0 ROOTFS_TAR OUTPUT_EXT4" >&2; exit 2; fi
command -v mkfs.ext4 >/dev/null
script_dir=$(cd -- "$(dirname -- "$0")" && pwd)
rootfs_source=$(realpath -- "$1")
output=$(realpath -m -- "$2")
if [[ -e "$output" ]]; then echo "Output already exists; choose a new image path" >&2; exit 2; fi
staging=$(mktemp -d)
trap 'rm -rf -- "$staging"' EXIT
# Input must be a trusted Docker export, not an uploaded agent artifact.
tar -xf "$rootfs_source" -C "$staging"
mkdir -p "$staging/opt/dasein" "$staging/workspace" "$staging/proc" "$staging/sys" "$staging/dev" "$staging/tmp"
install -m 0644 "$script_dir/agent.py" "$staging/opt/dasein/agent.py"
install -m 0755 "$script_dir/init.sh" "$staging/sbin/dasein-init"
printf 'protocol=1\n' > "$staging/etc/dasein-firecracker-guest"
# No setuid/setgid executables in the guest image.
find "$staging" -type f -perm /6000 -exec chmod a-s {} +
truncate -s 256M "$output"
mkfs.ext4 -q -F -O '^metadata_csum_seed' -d "$staging" "$output"
chmod 0444 "$output"
sha256sum "$output"
