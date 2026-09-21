# syntax=docker/dockerfile:1

FROM node:24-bookworm-slim AS frontend
WORKDIR /app/client
COPY client/package.json client/package-lock.json ./
RUN npm ci
COPY client/ ./
RUN npm run build

FROM rust:1.96-bookworm AS chef
WORKDIR /app
COPY rust-toolchain.toml ./
RUN cargo install cargo-chef --locked

FROM chef AS planner
COPY Cargo.toml Cargo.lock build.rs LICENSE THIRD_PARTY_NOTICES ./
COPY src/ src/
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS backend
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --locked --release --recipe-path recipe.json
COPY Cargo.toml Cargo.lock build.rs LICENSE THIRD_PARTY_NOTICES ./
COPY src/ src/
COPY migrations/ migrations/
COPY --from=frontend /app/client/dist ./client/dist
ARG ORCA_SOURCE_URL
RUN ORCA_SOURCE_URL="$ORCA_SOURCE_URL" cargo build --locked --release

FROM backend AS rust-sources
RUN mkdir -p /sources /notices \
    && cargo vendor --locked vendor > /sources/vendor-config.toml \
    && mv vendor /sources/ \
    && cp Cargo.toml Cargo.lock /sources/ \
    && tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner -czf /OrcaServer-dependencies.tar.gz -C /sources . \
    && cd /sources/vendor \
    && find . -type f \( -iname '*license*' -o -iname '*copying*' -o -iname '*copyright*' -o -iname 'notice*' \) \
       -exec cp --parents '{}' /notices/ \;

FROM ubuntu:24.04 AS slicer
RUN test "$(dpkg --print-architecture)" = amd64
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build
ADD --checksum=sha256:d12fb8c8eac1aecd2dfb6377acd48f994f8fa439ed5292fa532dd82880f029fd \
    https://github.com/OrcaSlicer/OrcaSlicer/releases/download/v2.4.2/OrcaSlicer_Linux_AppImage_Ubuntu2404_V2.4.2.AppImage OrcaSlicer.AppImage
RUN chmod +x OrcaSlicer.AppImage && ./OrcaSlicer.AppImage --appimage-extract >/dev/null
ADD --checksum=sha256:889a804661bcc839cecb7e5c3aa8a5680d5d8a361c40b68333add06e96819837 \
    https://codeload.github.com/OrcaSlicer/OrcaSlicer/tar.gz/8500fcdccaa10b5099ac20d252af3a7c560046f1 OrcaSlicer-source.tar.gz
RUN mkdir source && tar xzf OrcaSlicer-source.tar.gz -C source --strip-components=1

FROM slicer AS source-bundle
RUN apt-get update && apt-get install -y --no-install-recommends git libarchive-tools && rm -rf /var/lib/apt/lists/*
COPY packaging/slicer-sources.txt /sources/slicer-sources.txt
COPY packaging/fetch-slicer-sources.sh /build/fetch-slicer-sources.sh
RUN sh /build/fetch-slicer-sources.sh
COPY --from=slicer /build/OrcaSlicer-source.tar.gz /sources/OrcaSlicer-2.4.2-source.tar.gz
RUN tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner -czf /OrcaSlicer-2.4.2-sources.tar.gz -C /sources .
RUN mkdir -p /notices /expanded \
    && cp -a /build/source /expanded/OrcaSlicer \
    && cp -a /sources/wxWidgets /expanded/wxWidgets \
    && for archive in /sources/dependencies/*; do \
         directory="/expanded/$(basename "$archive")"; mkdir "$directory"; \
         bsdtar -xf "$archive" -C "$directory"; \
       done \
    && cd /expanded \
    && find . -type f \( -iname '*license*' -o -iname '*copying*' -o -iname '*copyright*' -o -iname 'notice*' \) \
       -exec cp --parents '{}' /notices/ \;

FROM scratch AS sources
COPY --from=source-bundle /OrcaSlicer-2.4.2-sources.tar.gz /
COPY --from=rust-sources /OrcaServer-dependencies.tar.gz /

FROM ubuntu:24.04 AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl libwebkit2gtk-4.1-0 libglu1-mesa libopengl0 libsm6 libmspack0t64 \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
LABEL org.opencontainers.image.licenses="AGPL-3.0-only"
COPY LICENSE THIRD_PARTY_NOTICES /usr/share/doc/orca-server/
COPY --from=rust-sources /notices /usr/share/doc/orca-server/dependencies
COPY --from=frontend /app/client/node_modules/svelte/LICENSE.md /usr/share/doc/orca-server/frontend/svelte-LICENSE
COPY --from=frontend /app/client/node_modules/esm-env/LICENSE /usr/share/doc/orca-server/frontend/esm-env-LICENSE
COPY --from=frontend /app/client/node_modules/clsx/license /usr/share/doc/orca-server/frontend/clsx-LICENSE
COPY --from=frontend /app/client/node_modules/normalize.css/LICENSE.md /usr/share/doc/orca-server/frontend/normalize-LICENSE
COPY --from=backend /app/target/release/orca-server /usr/local/bin/orca-server
COPY --from=slicer /build/squashfs-root /opt/orcaslicer
COPY --from=slicer /build/source/LICENSE.txt /usr/share/doc/orcaslicer/LICENSE
COPY packaging/slicer-sources.txt /usr/share/doc/orcaslicer/
COPY --from=source-bundle /notices /usr/share/doc/orcaslicer/notices
COPY --from=slicer /build/source/src/slic3r/GUI/AboutDialog.cpp /usr/share/doc/orcaslicer/AboutDialog.cpp
ENV PORT=3000
ENV PLATES_DIR=/data/plates
ENV ORCA_APPDIR=/opt/orcaslicer
RUN rm -rf /opt/orcaslicer/lib/orca-runtime \
    && mkdir -p /data/plates && chown -R 10001:10001 /data \
    && dpkg-query -W -f '${binary:Package}\t${Version}\t${source:Package}\t${source:Version}\n' \
       > /usr/share/doc/orca-server/system-packages.txt
EXPOSE 3000
USER 10001:10001
HEALTHCHECK --interval=30s --timeout=3s --start-period=15s \
    CMD curl --fail --silent "http://127.0.0.1:${PORT}/healthz" || exit 1
ENTRYPOINT ["orca-server"]
