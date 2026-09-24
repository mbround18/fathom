#!/bin/sh
# Lab only. Registers Dex (LDAP bridge) in Rauthy as an upstream OIDC provider and creates the
# Secret the group reconciler uses. Requires the `ldap-lab` API key from k8s/rauthy-clients
# (AuthProviders/Users/Groups) which is seeded on a FRESH Rauthy data dir (recreate the rauthy pod
# after `kubectl apply -k k8s/rauthy-clients`).
#
# Browser note: the Dex issuer is the in-cluster URL. To click through the login from the host add
#   127.0.0.1 dex.ldap-lab.svc.cluster.local
# to /etc/hosts and run: kubectl -n ingress-nginx port-forward svc/ingress-nginx-controller 8081:80
# (or rely on the reconciler + admin API for headless checks).
set -eu
CTX="${CTX:-kind-fathom-oidc-rauthy}"
KEY='ldap-lab$ldaplabgroupwriterkeysecretldaplabgroupwriterkeysecretldaplabgroupw'
DEX="http://dex.ldap-lab.svc.cluster.local:5556/dex"
K="kubectl --context $CTX"

# call Rauthy from inside the cluster (its ingress is https with a lab CA)
rauthy() { # method path [json]
  $K -n ldap-lab run "rc-$$-$(date +%s%N | tail -c 6)" --rm -i --restart=Never -q --image=curlimages/curl -- \
    curl -s -X "$1" -H "Authorization: API-Key $KEY" -H 'Content-Type: application/json' \
    ${3:+-d "$3"} "http://rauthy.rauthy.svc.cluster.local:8080$2"
}

body=$(cat <<EOF
{"name":"Fathom LDAP (Dex)","typ":"custom","enabled":true,"issuer":"$DEX",
 "authorization_endpoint":"$DEX/auth","token_endpoint":"$DEX/token","userinfo_endpoint":"$DEX/userinfo",
 "client_id":"rauthy","secret":"rauthy-dex-lab-secret","scope":"openid email profile groups",
 "use_pkce":false,"client_secret_basic":true,"client_secret_post":false,
 "admin_claim_path":null,"admin_claim_value":null,"mfa_claim_path":null,"mfa_claim_value":null}
EOF
)
echo "registering upstream provider..."
rauthy POST /auth/v1/providers "$body"
echo
$K -n ldap-lab create secret generic rauthy-group-writer --from-literal=api-key="$KEY" \
  --dry-run=client -o yaml | $K apply -f -
echo "done. Run the reconciler after the first LDAP login:"
echo "  $K -n ldap-lab create job --from=cronjob/group-reconciler r1"
