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

`OwnedIdentity::Prepare` requires public-validation callbacks from the shared PKI
validator, one reviewed client leaf, exact canonical HTTPS origin bindings, an
encrypted PKCS#12, a fresh 256-bit supervisor enrollment generation, and a bounded
native password. Public-material reads and signing handshakes must present that exact
generation, so commands retained across store replacement cannot address the new
identity. It wipes the supplied password on
every exit. It creates an explicit file Keychain and passes that retained store to
`SecItemImport`, with sign-only usage and an explicit permanent/sensitive attribute
array that omits extractability. Imported chain certificates grant no CA trust.
The retained key is checked against its store and nonextractable reference-key
attributes; its private bytes and native key reference never leave this class.
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

Compile the OFF-default Debug library and its file-only regression target:

```sh
cmake -S native/browser/pki/mac -B .local/mac-owned-pki-source \
  -DCMAKE_BUILD_TYPE=Debug -DCOLOSSUS_MAC_OWNED_PKI_PROTOTYPE=ON
cmake --build .local/mac-owned-pki-source --target \
  colossus_mac_owned_identity colossus-mac-owned-pki-file-test
node scripts/development-launch.mjs -- \
  .local/mac-owned-pki-source/colossus-mac-owned-pki-file-test
```

The thirteen file-only regressions exercise the real cleanup state machine with plain
owned files, plus password wiping and memory revocation. They also prove that a
plausible derived Keychain lock basename and a later database inode generation are
rejected and preserved rather than adopted, and that stale enrollment generations,
foreign origins and other leaf fingerprints fail the binding check. Stale-generation
handshake cancellation is rejected, and revocation or cleanup erases the retained
generation. They construct no native key/item/code references and invoke no Security,
browser or audit-port APIs.
This target does not run a real store or TLS acceptance fixture. Accepting native
output still requires an independently enforced creator-only mutation window and
exact generation receipt; the library intentionally has neither.
