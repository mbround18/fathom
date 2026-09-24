#!/usr/bin/env bash
# T032 static checks for the in-cluster daemon manifests (spec 036). No cluster needed.
#   k8s/test-daemon-static.sh        needs: kubectl (kustomize), python3 + PyYAML; kubeconform optional
set -euo pipefail
cd "$(dirname "$0")"
for ov in daemon daemon-namespaced; do
  kubectl kustomize "overlays/$ov" > "${TMPDIR:-/tmp}/fathom-$ov.yaml"
  if command -v kubeconform >/dev/null; then
    kubeconform -strict -ignore-missing-schemas -summary "${TMPDIR:-/tmp}/fathom-$ov.yaml"
  else
    echo "kubeconform not installed: schema validation skipped for $ov"
  fi
  python3 - "${TMPDIR:-/tmp}/fathom-$ov.yaml" "$ov" <<'PY'
import sys, re, yaml
docs = [d for d in yaml.safe_load_all(open(sys.argv[1])) if d]
ov = sys.argv[2]
fail = []
READ = {"get", "list", "watch"}
for d in docs:
    k = d["kind"]
    if k in ("ClusterRole", "Role"):
        for r in d["rules"]:
            res, verbs = set(r.get("resources", [])), set(r["verbs"])
            if res == {"subjectaccessreviews"} and verbs == {"create"} and k == "ClusterRole":
                continue
            if k == "Role" and d["metadata"].get("namespace") == "kube-system":
                if not (r.get("resourceNames") == ["extension-apiserver-authentication"] and verbs == {"get"}):
                    fail.append(f"kube-system Role rule too broad: {r}")
                continue
            if not verbs <= READ:
                fail.append(f"{k}/{d['metadata']['name']} non-read verbs {verbs} on {res}")
            if res & {"secrets", "configmaps", "*"}:
                fail.append(f"{k}/{d['metadata']['name']} touches {res}")
kinds = {d["kind"] for d in docs}
if "APIService" not in kinds:
    fail.append("no APIService rendered")
api = [d for d in docs if d["kind"] == "APIService"][0]
if api["metadata"]["name"] != "v1.daemon.fathom.dev":
    fail.append("APIService name")
# uninstall selector coverage: every object labeled, and every non-workload kind (cluster-scoped, or namespaced
# outside fathom-daemon such as the kube-system Role) is in cluster.sh's `delete <kinds> -A -l` list
line = [l for l in open("cluster.sh") if "delete apiservice" in l][0]
covered = {t.lower() for t in re.search(r"delete (\S+)", line).group(1).split(",")}
lbl = "app.kubernetes.io/name"
for d in docs:
    m = d["metadata"]
    if m.get("labels", {}).get(lbl) != "fathom-daemon":
        fail.append(f"{d['kind']}/{m['name']} missing uninstall label")
    ns = m.get("namespace")
    if ns != "fathom-daemon" and d["kind"].lower() not in covered:
        fail.append(f"{d['kind']}/{m['name']} (ns={ns}) not covered by uninstall kinds {covered}")
    if d["kind"] == "Deployment":
        sp = d["spec"]["template"]["spec"]
        c = sp["containers"][0]; cs = c["securityContext"]
        if not (sp["securityContext"]["runAsNonRoot"] and sp["securityContext"]["seccompProfile"]["type"] == "RuntimeDefault"
                and cs["readOnlyRootFilesystem"] and cs["capabilities"]["drop"] == ["ALL"] and not cs["allowPrivilegeEscalation"]):
            fail.append("Deployment securityContext hardening incomplete")
        if d["spec"]["replicas"] != 1:
            fail.append("replicas != 1")
if fail:
    print(f"[{ov}] FAIL"); [print("  -", f) for f in fail]; sys.exit(1)
print(f"[{ov}] ok: {len(docs)} objects")
PY
done
