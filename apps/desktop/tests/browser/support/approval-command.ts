// Credentials stay in an inert comment; only the optional delay and marker execute.
export function approvalMarkerCommand(
  windows: boolean,
  delayed = false,
): string {
  // Prove the collector observes a command beyond its former ten-second budget.
  const delay = delayed
    ? windows
      ? "Start-Sleep -Seconds 12; "
      : "sleep 12; "
    : "";
  const marker = windows
    ? "Add-Content -LiteralPath 'approved-marker.txt' -Value 'approved' -Encoding Ascii"
    : "printf 'approved\\n' >> approved-marker.txt";
  const credentials =
    'TOKEN=fixture-private-token Bearer AZ~fixture-bearer-suffix== PASSWORD=top"fixture-concat-tail" --oauth2-"bearer" fixture-name-tail ssh-keygen -N fixture-keygen-tail keytool -storepass fixture-storepass-tail openssl cms -pwri_password fixture-pwri-tail -----BEGIN PGP PRIVATE KEY BLOCK----- fixture-openpgp-tail -----END PGP PRIVATE KEY BLOCK-----';
  return `${delay}${marker} # ${credentials} ${"x".repeat(windows ? 1400 : 6000)} COMMAND_TAIL`;
}
