// Credentials stay in an inert comment; only the delay and phase markers execute.
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
  // Use core PowerShell/.NET file I/O, without cmdlet module autoload. These
  // marker writes still run inside the ordinary permit-bound process effect.
  const marker = windows
    ? "[System.IO.File]::AppendAllText('approved-marker.txt', 'approved' + [char]13 + [char]10)"
    : "printf 'approved\\n' >> approved-marker.txt";
  const started = windows
    ? "[System.IO.File]::AppendAllText('started-marker.txt', 'started' + [char]13 + [char]10); "
    : "printf 'started\\n' >> started-marker.txt; ";
  const credentials =
    'TOKEN=fixture-private-token Bearer AZ~fixture-bearer-suffix== PASSWORD=top"fixture-concat-tail" --oauth2-"bearer" fixture-name-tail ssh-keygen -N fixture-keygen-tail keytool -storepass fixture-storepass-tail openssl cms -pwri_password fixture-pwri-tail -----BEGIN PGP PRIVATE KEY BLOCK----- fixture-openpgp-tail -----END PGP PRIVATE KEY BLOCK-----';
  return `${started}${delay}${marker} # ${credentials} ${"x".repeat(windows ? 1400 : 6000)} COMMAND_TAIL`;
}
