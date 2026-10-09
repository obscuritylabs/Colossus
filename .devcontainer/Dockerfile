# Only rustup proxies are copied; mise installs the canonical compiler below.
FROM rust:1.96.0-bookworm@sha256:5e2214abe154fe26e39f64488952e5c991eeed1d6d6da7cc8381ae83927f0cfc AS rustup

# mcr.microsoft.com/devcontainers/base:2-bookworm
FROM mcr.microsoft.com/devcontainers/base@sha256:bb7b81b6e5be17b5267f92f4ffda534fea37dab1df97b5e86c1f9b91da5c0b5d

USER root
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
    && rm -rf /var/lib/apt/lists/*

COPY scripts/ci/mise-bootstrap.sh /tmp/mise-bootstrap.sh
RUN case "$(uname -m)" in \
        x86_64) arch=X64 ;; aarch64) arch=ARM64 ;; *) exit 1 ;; \
    esac \
    && sh /tmp/mise-bootstrap.sh Linux "$arch" --install /usr/local/bin/mise \
    && rm /tmp/mise-bootstrap.sh

ENV CARGO_HOME=/home/vscode/.cargo \
    RUSTUP_HOME=/home/vscode/.rustup \
    MISE_DATA_DIR=/home/vscode/.local/share/mise \
    MISE_ENV=devcontainer
ENV PATH="/home/vscode/.cargo/bin:/home/vscode/.local/share/mise/shims:${PATH}"
COPY --from=rustup --chown=vscode:vscode /usr/local/cargo/bin/ /home/vscode/.cargo/bin/
# Seed rustup metadata so mise reuses the digest-verified proxies.
COPY --from=rustup --chown=vscode:vscode /usr/local/rustup/settings.toml /home/vscode/.rustup/
COPY --chown=vscode:vscode mise.toml mise.lock rust-toolchain.toml /opt/colossus-toolchain/
COPY --chown=vscode:vscode mise.devcontainer.toml mise.devcontainer.lock /opt/colossus-toolchain/

USER vscode
WORKDIR /opt/colossus-toolchain
RUN mise trust \
    && mise install --locked \
    && mise reshim \
    && mise exec rust -- rustup default "$(mise current rust)" \
    && mise exec rust -- rustup component add rust-analyzer rust-src
# Global filenames let mise discover both the inventory and its environment.
RUN ln -s mise.toml config.toml \
    && ln -s mise.devcontainer.toml config.devcontainer.toml
ENV MISE_CONFIG_DIR=/opt/colossus-toolchain
WORKDIR /home/vscode
