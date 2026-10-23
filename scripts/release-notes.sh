#!/usr/bin/env bash
# Render the release notes for a tag from .github/RELEASE_TEMPLATE.md:
#   - the CHANGELOG.md section for the version (prerelease suffix stripped: v0.1.0-rc.1 -> 0.1.0)
#   - a table of the assets currently on the GitHub release
#   - how to verify them: attestations when release.yml built them, checksums and the signed tag
#     when they were built locally (detected from the attestation API)
#   - GitHub's auto-generated "what's changed" (commits/PRs since the previous tag)
#
#   scripts/release-notes.sh v0.1.0-rc.1            # markdown to stdout
#   scripts/release-notes.sh v0.1.0-rc.1 --apply    # also write it to the release
#
# Needs `gh` logged in with repo access. Used by the publish job in release.yml and by hand.
set -euo pipefail
cd "$(dirname "$0")/.."

tag=${1:?usage: release-notes.sh <tag> [--apply]}
apply=${2:-}
repo=${GITHUB_REPOSITORY:-$(gh repo view --json nameWithOwner -q .nameWithOwner)}
version=${tag#v}; version=${version%%-*}

changelog=$(awk -v v="$version" '
  /^## \[/ { on = ($0 ~ "^## \\[" v "\\]") ; if (on) { print "## Changes in " v; next } }
  on { print }' CHANGELOG.md)
[ -n "$changelog" ] || changelog="_No CHANGELOG.md section for $version yet._"

prerelease_note=""
case "$tag" in
  *-*) prerelease_note="> **Pre-release** \`$tag\`: for testing. Expect rough edges and report them in [issues](https://github.com/$repo/issues)." ;;
esac

assets=$(gh release view "$tag" --repo "$repo" --json assets -q '.assets[].name' | awk '
  /\.deb$/      { printf "| Debian / Ubuntu | `%s` | validated |\n", $0; next }
  /\.rpm$/      { printf "| Fedora / RHEL | `%s` | validated |\n", $0; next }
  /\.AppImage$/ { printf "| Any Linux distro | `%s` | validated |\n", $0; next }
  /-setup\.exe$/ { printf "| Windows x64 | `%s` | experimental, untested |\n", $0; next }
  /\.msi$/      { printf "| Windows x64 (MSI) | `%s` | experimental, untested |\n", $0; next }
  /\.dmg$/      { printf "| macOS | `%s` | experimental, untested |\n", $0; next }
  /^SHA256SUMS$/ { printf "| Checksums | `%s` | |\n", $0; next }
  /^fathom-desktop$/ { printf "| Bare Linux binary | `%s` | validated |\n", $0; next }
  { printf "| | `%s` | |\n", $0 }')
[ -n "$assets" ] || assets="| | _(no assets uploaded yet)_ | |"

# Attested when GitHub holds a provenance attestation for the first digest in the release's
# SHA256SUMS (release.yml attests every asset before this runs). A local build has none.
attested=0
if sums=$(gh release download "$tag" --repo "$repo" --pattern SHA256SUMS --output - 2>/dev/null); then
  digest=$(awk 'NR == 1 { print $1 }' <<<"$sums")
  if [ -n "$digest" ] &&
    gh api "repos/$repo/attestations/sha256:$digest" -q '.attestations | length' >/dev/null 2>&1; then
    attested=1
  fi
fi
echo "release-notes: assets $([ "$attested" = 1 ] && echo attested || echo 'not attested (local build)')" >&2

auto=$(gh api --method POST "repos/$repo/releases/generate-notes" -f tag_name="$tag" -q .body 2>/dev/null || true)
auto=${auto//"## What's Changed"/"## What's changed since the previous tag"}

export PRERELEASE_NOTE="$prerelease_note" CHANGELOG="$changelog" ASSETS="$assets" AUTO_NOTES="$auto"
out=$(python3 -I - "$tag" "$version" "$attested" <<'PY'
import sys, os, re
tag, version, attested = sys.argv[1], sys.argv[2], sys.argv[3] == "1"
t = open(".github/RELEASE_TEMPLATE.md", encoding="utf-8").read()
t = "\n".join(l for l in t.splitlines() if not l.startswith("<!--")).strip() + "\n"
keep, drop = ("#", "^") if attested else ("^", "#")
section = r"\{\{%sATTESTED\}\}\n(.*?)\{\{/ATTESTED\}\}\n"
t = re.sub(section % re.escape(keep), r"\1", t, flags=re.S)
t = re.sub(section % re.escape(drop), "", t, flags=re.S)
for k in ("PRERELEASE_NOTE", "CHANGELOG", "ASSETS", "AUTO_NOTES"):
    t = t.replace("{{" + k + "}}", os.environ[k])
t = t.replace("{{TAG}}", tag).replace("{{VERSION}}", version)
print(re.sub(r"\n{3,}", "\n\n", t).strip())
PY
)

if [ "$apply" = "--apply" ]; then
  gh release edit "$tag" --repo "$repo" --notes-file - <<<"$out"
  echo "release notes written to $tag" >&2
else
  printf '%s\n' "$out"
fi
