FROM python:3.12-slim
COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /bin/

WORKDIR /src

# Copy only project metadata first for better caching
COPY . ./

# Install build tools and build a wheel
RUN uv sync --all-groups && uv build

RUN apt-get update && \
    apt-get install -y gzip && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/* && \
    tar -czvf /colossus.tar.gz /src/

# Default to running the development CLI via `uv run colossus`.
ENTRYPOINT ["uv", "run", "colossus"]