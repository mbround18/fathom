#!/bin/sh
# IdP-side group reconciler (no Fathom code). Mirrors LDAP group membership into Rauthy groups:
# for every LDAP user that already exists in Rauthy (matched by email, i.e. after first upstream
# login), set Rauthy `groups` to the user's LDAP groups. Rauthy 0.36.2 has no upstream group mapping.
set -eu
RAUTHY="${RAUTHY_URL:-http://rauthy.rauthy.svc.cluster.local:8080}"
AUTH="Authorization: API-Key ${RAUTHY_API_KEY}"
BASE="dc=fathom,dc=test"
ldap() { ldapsearch -x -LLL -H "$LDAP_URL" -D "cn=admin,$BASE" -w "$LDAP_ADMIN_PASSWORD" -b "$@"; }

# ensure every LDAP group exists in Rauthy
for g in $(ldap "ou=groups,$BASE" '(objectClass=groupOfNames)' cn | sed -n 's/^cn: //p'); do
  curl -s -H "$AUTH" -H 'Content-Type: application/json' -d "{\"group\":\"$g\"}" "$RAUTHY/auth/v1/groups" >/dev/null || true
done

rauthy_users=$(curl -sf -H "$AUTH" "$RAUTHY/auth/v1/users")
for uid in $(ldap "ou=people,$BASE" '(objectClass=inetOrgPerson)' uid | sed -n 's/^uid: //p'); do
  email=$(ldap "ou=people,$BASE" "(uid=$uid)" mail | sed -n 's/^mail: //p')
  id=$(echo "$rauthy_users" | jq -r --arg e "$email" '.[] | select(.email==$e) | .id' | head -n1)
  [ -n "$id" ] || { echo "skip $uid ($email): not yet in Rauthy"; continue; }
  groups=$(ldap "ou=groups,$BASE" "(member=uid=$uid,ou=people,$BASE)" cn | sed -n 's/^cn: //p' | jq -R . | jq -sc .)
  cur=$(curl -sf -H "$AUTH" "$RAUTHY/auth/v1/users/$id")
  body=$(echo "$cur" | jq -c --argjson g "$groups" '{email, given_name, family_name, language, roles, groups: $g, enabled, email_verified}')
  code=$(curl -s -o /dev/null -w '%{http_code}' -X PUT -H "$AUTH" -H 'Content-Type: application/json' -d "$body" "$RAUTHY/auth/v1/users/$id")
  echo "reconciled $uid -> $groups (HTTP $code)"
done
