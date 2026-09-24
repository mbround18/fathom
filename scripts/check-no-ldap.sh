#!/bin/sh
# Spec 033 guard: Fathom must not depend on any LDAP client crate (LDAP is bridged by Dex + Rauthy).
set -eu
if cargo tree --workspace --all-features --prefix none 2>/dev/null | grep -i -E '(^|[^a-z])(ldap|openldap)'; then
  echo "error: an LDAP crate is in the dependency tree (see above)" >&2
  exit 1
fi
echo "ok: no LDAP crate in cargo tree"
