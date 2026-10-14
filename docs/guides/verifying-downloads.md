# Verifying a Fathom download

Fathom bundles are not code-signed (no Authenticode certificate for Windows, no Apple notarization).
Windows SmartScreen and some Linux app stores will warn about that. Instead, every file on a
[release](https://github.com/mbround18/fathom/releases) built by GitHub Actions carries two things
you can check before running it:

1. a **SHA256 checksum** in the `SHA256SUMS` asset, and
2. a **build provenance attestation**: a Sigstore signature, issued by GitHub, that ties the exact
   bytes of the file to the Fathom repository, the release workflow and the commit that built it.

The attestation is the stronger check. It proves the file came out of
`.github/workflows/release.yml` in `mbround18/fathom`, not from someone else's machine. A checksum
only proves the download was not corrupted.

## Verify the provenance (recommended)

You need the [GitHub CLI](https://cli.github.com/) 2.49 or newer, logged in (`gh auth login`).

```sh
gh attestation verify Fathom_0.1.0_x64-setup.exe --repo mbround18/fathom
```

The same command works for every asset: `.deb`, `.rpm`, `.AppImage` and the `.exe`. A good result
looks like:

```
Loaded digest sha256:… for file://Fathom_0.1.0_x64-setup.exe
Loaded 1 attestation from GitHub API
✓ Verification succeeded!

sha256:…  was attested by:
REPO             PREDICATE_TYPE                  WORKFLOW
mbround18/fathom https://slsa.dev/provenance/v1  .github/workflows/release.yml@refs/tags/v0.1.0-rc.1
```

Anything else (`✗ Verification failed`, "no attestations found") means the file was modified after
the build, or was never built by the release workflow. Do not run it.

To see the full provenance (builder, commit, trigger):

```sh
gh attestation verify Fathom_0.1.0_x64-setup.exe --repo mbround18/fathom --format json
```

On Windows PowerShell the command is identical. If `gh` is not an option, the attestations are
public Sigstore bundles and can be checked with
[`cosign`](https://github.com/sigstore/cosign) or [`slsa-verifier`](https://github.com/slsa-framework/slsa-verifier)
after downloading them with `gh attestation download`.

## Verify the checksum

Download `SHA256SUMS` from the same release into the folder with your file, then:

```sh
sha256sum -c --ignore-missing SHA256SUMS      # Linux
```

```powershell
Get-FileHash .\Fathom_0.1.0_x64-setup.exe     # Windows: compare with the line in SHA256SUMS
```

## Which releases are attested

Only assets uploaded by the GitHub Actions release workflow have attestations. Files built and
uploaded by hand (as the first assets of `v0.1.0-rc.1` were, before Actions was enabled) have a
checksum but no attestation; the workflow re-run replaced them with attested builds. If
`gh attestation verify` reports no attestation for a current release asset, open an issue.
