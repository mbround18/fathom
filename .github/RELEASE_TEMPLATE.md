<!-- Rendered by scripts/release-notes.sh; the publish job writes the result to the release. -->
<!-- Placeholders: {{TAG}} {{VERSION}} {{PRERELEASE_NOTE}} {{CHANGELOG}} {{ASSETS}} {{AUTO_NOTES}} -->
{{PRERELEASE_NOTE}}

{{CHANGELOG}}

## Downloads

| Platform | Asset | Status |
|---|---|---|
{{ASSETS}}

Linux x86_64 is the validated target. The Windows installer is cross-built and attested but has
not been tested on Windows; macOS is not shipped yet.

## Verify your download

Nothing here is code-signed (no Authenticode certificate, no notarization), so Windows SmartScreen
will warn. Instead every asset carries a Sigstore build-provenance attestation issued by GitHub.
Check it with the [GitHub CLI](https://cli.github.com/) before installing:

```sh
gh attestation verify <asset> --repo mbround18/fathom
```

`✓ Verification succeeded!` means the file is byte-for-byte what `.github/workflows/release.yml`
built from tag `{{TAG}}`. Checksums for every asset are in `SHA256SUMS`
(`sha256sum -c --ignore-missing SHA256SUMS`). Details:
[docs/guides/verifying-downloads.md](https://github.com/mbround18/fathom/blob/main/docs/guides/verifying-downloads.md).

{{AUTO_NOTES}}
