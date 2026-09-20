#!/bin/sh
set -eu
# The fixed manifest comes from OrcaSlicer 2.4.2 deps/*.cmake.
mkdir -p /sources/dependencies
while read -r checksum filename url; do
    curl --fail --location --retry 3 --output "/sources/dependencies/$filename" "$url"
    printf '%s  %s\n' "$checksum" "/sources/dependencies/$filename" | sha256sum -c -
done < /sources/slicer-sources.txt
# The v3.3.2 branch at the AppImage's release date; include its submodules.
git clone --quiet --depth 1 --branch orca-3.3.2 --recurse-submodules --shallow-submodules \
    https://github.com/SoftFever/Orca-deps-wxWidgets.git /sources/wxWidgets
test "$(git -C /sources/wxWidgets rev-parse HEAD)" = db1005db3dea2c37a46fb455a9a02e37aa360751
git -C /sources/wxWidgets submodule status --recursive > /sources/wxWidgets-submodules.txt
find /sources/wxWidgets -name .git -prune -exec rm -rf '{}' +
