# Reproducible Linux Desktop package/acceptance builder, without host credentials.
FROM node:22.18.0-bookworm-slim@sha256:752ea8a2f758c34002a0461bd9f1cee4f9a3c36d48494586f60ffce1fc708e0e AS node
FROM rust:1.96.0-bookworm@sha256:5e2214abe154fe26e39f64488952e5c991eeed1d6d6da7cc8381ae83927f0cfc AS build-base
COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm && ln -s ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx
RUN apt-get update && apt-get install -y --no-install-recommends \
    clang cmake pkg-config libssl-dev libsecret-1-dev libwebkit2gtk-4.1-dev libasound2-dev \
    libayatana-appindicator3-dev librsvg2-dev libxdo-dev patchelf \
    dbus-x11 gnome-keyring xvfb xauth xdotool python3 \
    && rm -rf /var/lib/apt/lists/*
ENV CARGO_BUILD_JOBS=4
WORKDIR /build

FROM build-base AS package
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    --mount=type=cache,target=/build/apps/desktop/src-tauri/target \
    npm ci --prefix apps/desktop --ignore-scripts --legacy-peer-deps --install-links \
    && git init --quiet \
    && COLOSSUS_DESKTOP_RELEASE_CHANNEL=developer_preview COLOSSUS_DESKTOP_TEAM_ID=UNSIGNED sh scripts/package-desktop-linux
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    --mount=type=cache,target=/build/apps/desktop/src-tauri/target \
    xvfb-run -a dbus-run-session -- sh scripts/test-desktop-linux-native
FROM scratch AS artifacts
COPY --from=package /build/dist/desktop-linux/ /
