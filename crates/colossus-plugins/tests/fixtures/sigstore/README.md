# Sigstore verification fixtures

These public test artifacts are copied unchanged from `sigstore-verify` and `sigstore-trust-root` 0.9.0,
part of [prefix-dev/sigstore-rust](https://github.com/prefix-dev/sigstore-rust),
licensed under Apache-2.0 (the same license as this repository).

- `cosign-v3.sigstore.json`: upstream `test_data/bundles/cosign-v3-blob.sigstore.json`.
- `cosign-v3.txt`: upstream `test_data/bundles/cosign-v3-blob.txt`.
- `trusted-root.json`: upstream `sigstore-trust-root/src/trusted_root.json`, including the timestamp authority used by the fixture.

The bundle carries public certificate, transparency-log and timestamp evidence.
The signer identity is a fixture, not a Colossus publisher or production trust
configuration. Tests verify the exact signed bytes and reject tampering; no
network lookup, skipped cryptographic check, or invented signature format is used.
