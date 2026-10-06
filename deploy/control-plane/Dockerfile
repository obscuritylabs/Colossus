# syntax=docker/dockerfile:1
FROM node:22.18.0-bookworm-slim@sha256:752ea8a2f758c34002a0461bd9f1cee4f9a3c36d48494586f60ffce1fc708e0e AS web
WORKDIR /source
COPY apps/ui apps/ui
COPY apps/web apps/web
ARG VITE_COLOSSUS_DOCUMENTATION_URL=/docs/
RUN cd apps/web && npm ci --ignore-scripts --legacy-peer-deps --install-links && \
    VITE_COLOSSUS_DOCUMENTATION_URL="$VITE_COLOSSUS_DOCUMENTATION_URL" npm run build

FROM rust:1.96.0-bookworm@sha256:5e2214abe154fe26e39f64488952e5c991eeed1d6d6da7cc8381ae83927f0cfc AS build
WORKDIR /source
COPY . .
ARG PROFILE=release
ARG BUILD_JOBS=2
ENV CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_DEV_INCREMENTAL=false
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/source/target \
    CARGO_BUILD_JOBS=${BUILD_JOBS} cargo build --locked --profile ${PROFILE} -p colossus-cloud-server --bin colossus-control-plane && \
    if [ "$PROFILE" = dev ]; then output=debug; else output="$PROFILE"; fi && \
    install -D target/${output}/colossus-control-plane /artifacts/colossus-control-plane && strip /artifacts/colossus-control-plane

FROM debian:bookworm-slim@sha256:3783cc01769c7b2b1b83a5c5ad96c815348e28ed7da68e2e3687004faa906251 AS runtime
RUN apt-get update && apt-get install --yes --no-install-recommends ca-certificates libdbus-1-3 libsecret-1-0 && rm -rf /var/lib/apt/lists/* && \
    groupadd --gid 10001 colossus && useradd --uid 10001 --gid colossus --home-dir /var/lib/colossus --create-home colossus && chmod 0700 /var/lib/colossus
COPY --from=build /artifacts/colossus-control-plane /usr/local/bin/
COPY --from=web /source/apps/web/dist/ /opt/colossus/web/
USER 10001:10001
ENV COLOSSUS_HOME=/var/lib/colossus
EXPOSE 8080 8443
ENTRYPOINT ["colossus-control-plane"]
CMD ["/etc/colossus/control-plane.json"]
