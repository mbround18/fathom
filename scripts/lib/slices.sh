# Which slices a change needs. Shared by scripts/ci.sh (containers, the full gate) and
# scripts/gate.sh (the host-side gate while iterating), so both agree on what a diff touches.
# Source it from the repository root.

# Crates shared by more than one app. A change to one runs the shared slice and every app that
# depends on it (slices_for below).
SHARED_CRATES="-p container-runtime -p audit-log -p web-auth -p web-passkeys -p oci-registry"

# slices_for <path>...: the slices a set of changed paths needs, one per line. Docs and specs
# need none; anything not claimed by an app or the shared code is Fathom's.
slices_for() {
  local f
  for f in "$@"; do
    case "$f" in
      specs/* | docs/* | *.md | .specify/* | .claude/* | LICENSE*) ;;
      apps/depths/* | crates/depths-*) echo depths ;;
      apps/keel/* | crates/keel-*) echo keel ;;
      crates/container-runtime/* | crates/audit-log/*) printf '%s\n' shared fathom depths ;;
      crates/web-auth/*) printf '%s\n' shared fathom depths keel ;;
      crates/web-passkeys/*) printf '%s\n' shared keel ;;
      crates/oci-registry/*) printf '%s\n' shared depths keel ;;
      mk/*) printf '%s\n' fathom depths keel ;;
      packages/*) printf '%s\n' shared fathom depths keel ;;
      Cargo.toml | Cargo.lock | deny.toml | rust-toolchain* | package.json | pnpm-lock.yaml \
        | pnpm-workspace.yaml | scripts/ci.sh | scripts/lib/slices.sh | .github/*)
        printf '%s\n' shared fathom depths keel ;;
      *) echo fathom ;;
    esac
  done
}

# changed_slices <base>: the slices that the diff against <base> touches, committed and
# uncommitted, one per line, sorted.
changed_slices() {
  { git diff --name-only "$(git merge-base "$1" HEAD)"; git ls-files --others --exclude-standard; } \
    | sort -u | { mapfile -t files; [ "${#files[@]}" -eq 0 ] || slices_for "${files[@]}"; } | sort -u
}
