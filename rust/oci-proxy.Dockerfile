FROM scratch

ARG TARGET_TRIPLE=x86_64-unknown-linux-musl
COPY target/${TARGET_TRIPLE}/release/colossus-oci-proxy /usr/local/bin/colossus-oci-proxy

USER 65532:65532
ENTRYPOINT ["/usr/local/bin/colossus-oci-proxy"]
