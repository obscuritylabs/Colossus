# syntax=docker/dockerfile:1
# Keep the canonical Zensical pin aligned with scripts/docs-site.
FROM zensical/zensical:0.0.50@sha256:a67f689607908b47b9979ff8213906477f78826e9f8565f012e06071f883e973 AS build
WORKDIR /source
ARG SOURCE_DATE_EPOCH=0
ENV SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}
# Bash runs the repository's canonical compatibility redirect generator.
RUN apk add --no-cache bash
COPY docs docs
COPY zensical.toml /source/zensical.source.toml
COPY documentation/legacy-routes.tsv /source/documentation/legacy-routes.tsv
COPY scripts/generate-doc-redirects /source/scripts/generate-doc-redirects
COPY deploy/documentation/build-config.py /source/deploy/documentation/build-config.py
RUN python deploy/documentation/build-config.py zensical.source.toml zensical.toml && \
    zensical build --clean --strict && \
    bash scripts/generate-doc-redirects documentation/legacy-routes.tsv site /docs && \
    python deploy/documentation/build-config.py --timestamps site && \
    test -s site/index.html && test -s site/search.json && \
    test -s site/assets/vendor/mermaid-11.15.0.min.js

FROM nginx:1.30.5-alpine@sha256:0985e772fb9f729e6fa0980da05fca5d9c468e870eed43071545afa9d2e27d94
ARG COLOSSUS_VERSION=development
ARG COLOSSUS_REVISION=unknown
ARG SOURCE_DATE_EPOCH=0
LABEL org.opencontainers.image.title="Colossus documentation" \
      org.opencontainers.image.description="Public Colossus documentation and local search" \
      org.opencontainers.image.source="https://github.com/obscuritylabs/Colossus" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.version="${COLOSSUS_VERSION}" \
      org.opencontainers.image.revision="${COLOSSUS_REVISION}"
COPY --from=build /source/site/ /opt/colossus/documentation/docs/
COPY LICENSE /usr/share/licenses/colossus/LICENSE
COPY deploy/documentation/nginx.conf /etc/nginx/nginx.conf
USER 10001:10001
EXPOSE 8080
STOPSIGNAL SIGQUIT
# Skip the official entrypoint's runtime configuration writes.
ENTRYPOINT ["nginx", "-g", "daemon off;"]
