# Contributing to Fathom

Developer setup, the build targets and the CI story live in the [README](README.md#build-from-source)
for now; this file holds the process notes that do not belong there.

## Cutting a release

1. Bump the version in `Cargo.toml`, `ui/package.json` and `ui/src-tauri/tauri.conf.json`, and
   finish the `## [x.y.z]` section in `CHANGELOG.md` (its text becomes the top of the release
   notes). Commit (signed).
2. Tag and push: `git tag -s vX.Y.Z -m vX.Y.Z && git push origin refs/tags/vX.Y.Z`. A `-rc.N`
   suffix makes it a pre-release.
3. `.github/workflows/release.yml` runs on the tag: one tauri-action build per platform uploads
   bundles to a draft release, the AppImage is repacked against host libwayland, every bundle gets
   a Sigstore provenance attestation, then the publish job regenerates `SHA256SUMS`, renders the
   notes and flips the draft.
4. If Actions did not fire (it has been disabled before), run it by hand on the existing tag:
   `gh workflow run Release --ref main -f tag=vX.Y.Z`.

### Release notes

Notes are rendered from `.github/RELEASE_TEMPLATE.md` by `scripts/release-notes.sh`, which fills
in the CHANGELOG section for the version, a table of the assets on the release, the verification
instructions and GitHub's auto-generated "what's changed". Edit the template to change every
future release; to redo the notes on an existing one:

```sh
make release-notes TAG=vX.Y.Z           # preview to stdout
make release-notes TAG=vX.Y.Z APPLY=1   # write to the release
```

### Building without Actions

`make build-linux`, `scripts/appimage-host-libs.sh dist/*.AppImage`, `make stage`, then
`gh release create vX.Y.Z --verify-tag dist/*` (add `--prerelease` for an rc). Assets uploaded by
hand have a checksum but no attestation, so prefer re-running the workflow when it is available.
`make build-windows-paws` cross-builds the Windows installer in a container.
