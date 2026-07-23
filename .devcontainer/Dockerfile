FROM node:22.18.0-bookworm-slim@sha256:752ea8a2f758c34002a0461bd9f1cee4f9a3c36d48494586f60ffce1fc708e0e AS node
FROM python:3.10.18-slim-bookworm@sha256:445b9efb2c047a7ccdb30d293fd6b1aa0f62a55062dc2b69051df91e10848749 AS python
FROM golang:1.25.0-bookworm@sha256:81dc45d05a7444ead8c92a389621fafabc8e40f8fd1a19d7e5df14e61e98bc1a AS go
RUN GOBIN=/opt/go-tools/bin \
    go install github.com/rhysd/actionlint/cmd/actionlint@v1.7.12

FROM rust:1.96.0-bookworm@sha256:5e2214abe154fe26e39f64488952e5c991eeed1d6d6da7cc8381ae83927f0cfc AS rust-tools
RUN cargo install cargo-deny --version 0.20.2 --locked --root /opt/cargo-tools
RUN cargo install cargo-audit --version 0.22.2 --locked --root /opt/cargo-tools

# mcr.microsoft.com/devcontainers/base:2-bookworm
FROM mcr.microsoft.com/devcontainers/base@sha256:bb7b81b6e5be17b5267f92f4ffda534fea37dab1df97b5e86c1f9b91da5c0b5d

USER root

COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
COPY --from=python /usr/local /usr/local
COPY --from=go /usr/local/go /usr/local/go
COPY --from=go /opt/go-tools/bin/actionlint /usr/local/bin/actionlint
COPY --from=rust-tools /opt/cargo-tools/bin/cargo-audit /usr/local/bin/cargo-audit
COPY --from=rust-tools /opt/cargo-tools/bin/cargo-deny /usr/local/bin/cargo-deny

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends \
        build-essential \
        clang \
        cmake \
        curl \
        file \
        libayatana-appindicator3-dev \
        libsecret-1-dev \
        libssl-dev \
        libwebkit2gtk-4.1-dev \
        libxdo-dev \
        openssl \
        pkg-config \
        librsvg2-dev \
        wget \
    && ln --symbolic ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
    && ln --symbolic ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx \
    && rm -rf /var/lib/apt/lists/*

ENV PATH="/usr/local/go/bin:${PATH}"

USER vscode
