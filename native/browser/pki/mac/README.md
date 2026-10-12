# Owned macOS broker identity prototype

This is an unaccepted native custody library for a separate TLS broker. The CEF
host does not link it. It adds no browser availability, OS trust, personal-store
discovery, or production/signing acceptance. The shared Rust PKI crate remains an
effect-free public-certificate validator.

The private supervisor must create and retain a fresh private parent outside every
Root/browser filesystem grant, admit the exact distinct broker executable, and
deliver the enrollment through authenticated native bootstrap. A mode0700 directory
and a caller-supplied code hash do not establish those admission boundaries by
themselves. `Bootstrap` is an internal composition contract, never an IPC decoder.
Parents containing login/System Keychain namespaces are rejected before creation:
Apple's [file-store creation policy](https://github.com/apple-oss-distributions/Security/blob/main/OSX/libsecurity_keychain/lib/StorageManager.cpp)
can register login-named paths even without an explicit search-list setter.

`OwnedIdentity::Prepare` requires an independently enforced in-process
`MutationAuthority`, public-validation callbacks from the shared PKI
validator, one reviewed client leaf, exact canonical HTTPS origin bindings, an
encrypted PKCS#12, a fresh 256-bit supervisor enrollment generation, and a bounded
native password. Public-material reads and signing handshakes must present that exact
generation, so commands retained across store replacement cannot address the new
identity. It wipes the supplied password on
every exit. It creates an explicit file Keychain and passes that retained store to
`SecItemImport`, with sign-only usage and an explicit permanent/sensitive attribute
array that omits extractability. Imported chain certificates grant no CA trust.
The retained key is checked against its store and nonextractable reference-key
attributes. Exact persistent references record the one private key and every
imported certificate; searches remain explicit to that owned Keychain and reject
unrecorded items. Its private bytes and native key reference never leave this class.
Apple documents the extractable default in
[SecItemImportExportKeyParameters](https://developer.apple.com/documentation/security/secitemimportexportkeyparameters/keyattributes).

Only public CA DER and the reviewed leaf can be copied. The broker must put the
explicit CA roots into its own TLS verifier and retain normal chain/hostname
validation. This library never calls default, search-list, or OS trust setters.
It currently publishes one client leaf and supports TLS 1.3 SHA256 transcripts with
P256/SHA256 or RSA-PSS/SHA256 signatures. General client-chain publication, CA-only
enrollment, TLS 1.2 and other signing schemes are not provided.

The local handshake token is one-use, bounded to thirty seconds and one outstanding
handshake. `CertificateVerify` constructs the fixed RFC8446 client message and uses
the nonexportable retained `SecKey`. The broker TLS engine must supply its own
authenticated handshake transcript after exact request-origin/lease admission.
No raw signing endpoint may be exposed to Root or browser processes. Revocation
drains signing under the owner mutex before exact inode-bound store retirement.
The deadline fences signature acceptance before and after the synchronous
Security operation. It cannot interrupt a hung Security service or native mutex;
Tokio timeout/abort cannot establish that native call completed. The admitted
supervisor must independently bound the broker process and retain unknown native
cleanup after a timeout or forced termination.
An unknown creation, replacement, ACL change or cleanup outcome preserves physical
state; neither early drop nor process exit recursively deletes it.

The key ACL uses public trusted-application APIs. Public
`SecTrustedApplicationCopyData` exposes the path representation rather than a
serialized exact code requirement. Source checks therefore do not establish actual
foreign-process ACL denial, matching Apple's
[public API implementation](https://github.com/apple-oss-distributions/Security/blob/main/OSX/libsecurity_keychain/lib/SecTrustedApplication.cpp).
Dedicated broker/Root/renderer denial, TLS use, protected
state masking, peer/replay fencing, and installed-component acceptance must be
demonstrated by the native composition. CEF's public certificate-selection callback
does not accept this signer: the TLS broker must own both TLS legs and enforce each
HTTP authority; CONNECT/SNI validation alone is insufficient.

Namespace creation and each store/import/retirement window must retain independent
exclusion of every foreign
writer throughout the native operation. A successful native return is followed by
an exact two-descriptor generation receipt for the database and its derived lock.
The owner hardens those files before secret import and accepts an atomic database
replacement only within a positively sealed import window. Failed operations,
failed receipts and unexplained lock replacement retain unknown physical state.
Mode0700, advisory locks, directory snapshots, recognized basenames and caller
assertions cannot implement the mandatory `MutationAuthority` by themselves.
The optional concrete `NativeMutationAuthority` now supplies retained descriptor
receipts backed by immutable native role-policy exclusion. Actual cross-process
custody acceptance and production supervision remain unaccepted.
Before `mkdirat`, namespace creation acquires parent-only exclusion, then seals
the exact created directory descriptor before adopting its identity. A missing
or rejected directory receipt cannot become store/import authority.
Cleanup also requires that exclusion through rename, exact inode removal and
parent fsync. Signing revocation erases the signing generation immediately; a
separate physical-custody generation remains only until verified cleanup, so
revocation does not discard the cleanup authority needed for retries.

The `colossus-native-browser-mac-identity` Rust package provides a narrow audited
FFI over this actual native owner. It shares the separate TLS broker workspace and
lockfile. Only the optional `macos-native-identity-development` feature links it in
the broker. `ClientIdentity::from_mac_native` installs a local rustls resolver;
after decrypted exact-origin admission, rustls signs through the native one-use
handshake token. It accepts only the fixed RFC8446 client CertificateVerify message
with a SHA256 transcript, selects TLS1.3 SHA256 cipher suites, and retains normal
upstream CA/hostname validation. The control protocol does not accept identities,
native pointers, mutation callbacks or signing commands. The broker publishes
only public downstream trust/leaf/pin and its private proxy bootstrap for CEF.

Compile the OFF-default Debug library and its file-only regression target:

```sh
cmake -S native/browser/pki/mac -B .local/mac-owned-pki-source \
  -DCMAKE_BUILD_TYPE=Debug -DCOLOSSUS_MAC_OWNED_PKI_PROTOTYPE=ON
cmake --build .local/mac-owned-pki-source --target \
  colossus_mac_owned_identity colossus-mac-owned-pki-file-test
node scripts/development-launch.mjs -- \
  .local/mac-owned-pki-source/colossus-mac-owned-pki-file-test
```

The eighteen file-only regressions exercise the real cleanup state machine with plain
owned files, plus password wiping and memory revocation. They also prove that a
plausible derived Keychain lock basename and a later database inode generation are
rejected and preserved rather than adopted, and that stale enrollment generations,
foreign origins and other leaf fingerprints fail the binding check. Stale-generation
handshake cancellation is rejected, and revocation or cleanup erases the retained
generation. Positive test-owned generation receipts also exercise permitted
database replacement, rejected namespace/output exclusion and receipts, cleanup exclusion after signing
revocation and preserved foreign lock
generations. They construct no native key/item/code references and invoke no Security,
browser or audit-port APIs.
This target does not run a real store or TLS acceptance fixture. Accepting native
output still requires an independently enforced creator-only mutation window and
actual exact generation receipts from the admitted supervisor. Compilation and
synthetic receipts do not establish that boundary.

The optional `macos-native-custody-development` TLS entry independently embeds
the same six-field public package trust used by the Core build, through the
build-only `COLOSSUS_TLS_BROKER_MAC_PACKAGE_TRUST` input. It also embeds the exact
`COLOSSUS_TLS_BROKER_PARENT_CDHASH`; neither expected code nor trust keys come
from runtime files, HTTP, browser messages or IPC. After fixed fd3 parent admission
and the private 64-byte bootstrap, custody derives its actual executable and
fixed package paths, verifies the signed `owned-process-host.json`, package
inventory, exact role code signatures and policy resource, then derives the
custody parent from that authenticated resource. Current parent audit identity
must match the independently compiled Core seal. No audit-session join or
received Mach right is involved.

Every fixed front applies the final OS policy before loading its body/CEF.
`custody_policy.c` clamps every positive file grant with an explicit exclusion
of the custody root and removes file-extension issuance grants. This includes
more-specific stock paths, named predicates and extension filters. The native
creator admits only its fixed Main and four sealed role images; network and
general utility share the generic front. The authority checks embedded local
policy/front source hashes against the signed component receipt. It retains
the exact parent inode, namespace inode, successful database generation and
derived-lock inode; import can accept a database replacement only inside its
one-use successful output window. Retirement can use the already admitted
generation after Core loss, while fresh creation/import cannot.

Preparation stays unavailable until a separately reviewed native five-role
report exists at the fixed `owned-process-custody-denials.json` path and its
digest is present in the independently signed descriptor's optional
`custody_denials_sha256`. It binds the exact component inventory, policy,
embedded local sources and ordered Main/renderer/GPU/network/utility code
identities, denied read/write/create/rename/unlink/link/extension operations
and actual direct-child zero-exit reaping. The source diagnostic does not mint
that acceptance report. The documented
[sandbox threat boundary](../../../../docs/develop/security-architecture.md#development-credential-authority)
excludes arbitrary same-user native applications and compromised native code;
mode bits do not provide same-user application exclusion.

Build-only source validation of the optional entry uses the same public compiler
trust and completed Core code hash as the actual development package:

```sh
COLOSSUS_TLS_BROKER_MAC_PACKAGE_TRUST="$MAC_BROWSER_ALLOCATION/compiler-trust.json" \
COLOSSUS_TLS_BROKER_PARENT_CDHASH="$MAC_BROWSER_PARENT_CDHASH" \
  cargo clippy --locked --manifest-path native/browser/tls-broker/Cargo.toml \
  --features macos-native-custody-development --all-targets -- -D warnings
cmake --build .local/mac-owned-pki-source --target \
  colossus-mac-custody-boundary-file-test
node scripts/development-launch.mjs -- \
  .local/mac-owned-pki-source/colossus-mac-custody-boundary-file-test
```

The new boundary target uses plain synthetic files and invokes no Security,
Keychain, browser, process-spawn or audit APIs. It checks stale generations,
sequence replay, duplicate receipts, extra entries, hard links, admitted
database replacement and unadmitted replacement denial. This does not prove
native certificate import, signing or cleanup. The dedicated entry currently
retains the authority; actual reviewed identity enrollment remains a separate
native composition requirement.

Source/protocol verification of the concrete native linkage does not create a store
or execute any native entry:

```sh
cargo clippy --locked --manifest-path native/browser/pki/mac/Cargo.toml \
  --all-targets -- -D warnings
cargo test --locked --manifest-path native/browser/tls-broker/Cargo.toml \
  --features macos-native-identity-development
```

The mTLS protocol tests use memory-only synthetic keys and certificates. Real
nonexportable signing, foreign-process key denial, item-generation ownership,
cleanup, parent crash and installed distribution acceptance remain separate native
requirements; production stays disabled.
