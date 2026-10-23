<!-- Rendered by scripts/release-notes.sh; the publish job writes the result to the release. -->
<!-- Placeholders: {{TAG}} {{VERSION}} {{PRERELEASE_NOTE}} {{CHANGELOG}} {{ASSETS}} {{AUTO_NOTES}} -->
<!-- Sections: {{#ATTESTED}}...{{/ATTESTED}} renders when the assets have GitHub attestations, {{^ATTESTED}}...{{/ATTESTED}} when they don't (a local build). -->
{{PRERELEASE_NOTE}}

{{CHANGELOG}}

## Downloads

| Platform | Asset | Status |
|---|---|---|
{{ASSETS}}

Linux x86_64 is the validated target. The Windows installer is cross-built from Linux but has
not been tested on Windows; macOS is not shipped yet.

## Verify your download

Nothing here is code-signed (no Authenticode certificate, no notarization), so Windows SmartScreen
will warn.

{{#ATTESTED}}
Every asset carries a Sigstore build-provenance attestation issued by GitHub. Check it with the
[GitHub CLI](https://cli.github.com/) before installing:

```sh
gh attestation verify <asset> --repo mbround18/fathom
```

`✓ Verification succeeded!` means the file is byte-for-byte what `.github/workflows/release.yml`
built from tag `{{TAG}}`. Checksums for every asset are in `SHA256SUMS`
(`sha256sum -c --ignore-missing SHA256SUMS`).
{{/ATTESTED}}
{{^ATTESTED}}
These files were built on the maintainer's machine, not by GitHub Actions, so they carry **no**
Sigstore build-provenance attestation: `gh attestation verify` will report that none was found.
What you can check:

- **Checksums**: `sha256sum -c --ignore-missing SHA256SUMS` confirms the download is intact. It
  does not prove who built the file, since the checksums come from this same page.
- **The tag**: `{{TAG}}` is a GPG-signed tag. `git tag -v {{TAG}}` in a clone checks it, and you
  can build that commit yourself with `make build-linux`.
{{/ATTESTED}}

Details:
[docs/guides/verifying-downloads.md](https://github.com/mbround18/fathom/blob/main/docs/guides/verifying-downloads.md).

{{AUTO_NOTES}}
