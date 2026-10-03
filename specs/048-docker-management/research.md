# Phase 0 Research: Docker and container-runtime management

**Feature**: [spec.md](./spec.md) | **Plan**: [plan.md](./plan.md) | **Date**: 2026-10-02

Every finding below was verified against the live runtime on the development machine
(Docker 29.8.1) rather than taken from documentation. Where a claim could not be verified it says
so.

---

## D1. How the backend reaches the runtime

**Decision**: invoke the runtime's command-line interface as a child process through the existing
mockable `Runner` trait in `src/backend/kind.rs`, requesting machine-readable output.

**Rationale**:

1. **FR-018 requires it.** The preview must be "a form the operator could reproduce themselves". A
   command line *is* that form. An HTTP call to `/v1.49/containers/{id}/stop` is not something an
   operator reproduces, so an API-based implementation would have to synthesise a fake command
   string for the preview — a string that is then not what actually ran. That is precisely the
   blackboxing the operator has ruled out, and it would make the preview a decoration rather than a
   guarantee.
2. **Principle III gets cheap.** `Runner` is already a trait with a mock used by tests like
   `docker_stopped_podman_running_prefers_podman`. Every inventory, preview and blast-radius test
   becomes a canned string in, an assertion out, with no daemon, no container and no network.
3. **Docker-compatible alternatives come free.** The probe already detects both runtimes; their
   command surfaces are near-identical, so no socket version negotiation or API-dialect handling
   is needed.
4. **No new dependency**, and no second way of talking to a subprocess in the codebase.

**Alternatives rejected**:

- **`bollard` (or any Engine-API client crate).** Richer typed access and native streaming, but it
  breaks the preview guarantee above, adds a dependency whose version is coupled to the daemon's
  API version, and requires its own socket discovery and permission handling. Rejected on FR-018
  first and dependency weight second.
- **Writing to the socket directly with the existing HTTP client.** All of bollard's downsides
  without its types.

**Consequence to accept**: output parsing, and one process spawn per batched query. D2 and D7
address both.

---

## D2. Getting machine-readable output, and one trap in it

**Decision**: use `--format '{{json .}}'` for list commands, which emits one JSON object per line.
For labels, do **not** use the list command's `Labels` field — use either a targeted
`{{.Label "<key>"}}` projection or a batched `docker inspect`.

**Verified**: `docker ps -a --format '{{json .}}'` returns one object per line, with
`ID`, `Image`, `State`, `HealthStatus`, `CreatedAt`, `Command`, `Names`, `Ports` and `Labels`.

**The trap**: in list output, `Labels` is a single flat string of `k=v` pairs joined by commas:

```text
"Labels":"com.docker.compose.config-hash=8bba83c...,com.docker.compose.container-number=1,
com.docker.compose.depends_on=app:service_started:false,com.docker.compose.image=sha256:c6d613..."
```

A label *value* may itself contain `,` or `=` — `com.docker.compose.depends_on` demonstrably does.
Splitting that string is therefore ambiguous and would misattribute a container's project under
some label sets. Two safe routes, both verified:

- **Targeted projection** — `docker ps -a --format '{{.ID}}|{{.Label "com.docker.compose.project"}}|{{.State}}'`
  returns the label value as its own field. Verified output includes `...|thunderforge-app|running`
  and `...|slam-usa|restarting`.
- **Batched inspect** — `docker inspect --format '{{.Id}} {{json .Config.Labels}}' <id> <id> ...`
  returns labels as a real JSON object per container, in one process spawn for many ids. Verified.

**Decision**: the list query uses targeted projections for the two labels that drive grouping and
the cluster link, and `inspect` is used in one batched call when the full label set is actually
needed. This keeps the common path to a single spawn and never parses the ambiguous flat string.

---

## D3. Blast radius for prune — Fathom computes it, because the runtime cannot

**Decision**: Fathom computes the removal set itself from its own inventory and issues *targeted
removals by identity*, rather than issuing a broad prune and reporting afterwards.

**Verified**: `docker image prune --help` offers `--filter` and `--force` but **no `--dry-run`**.
There is no way to ask the runtime what a prune would take.

**Why this settles several requirements at once**:

- FR-023 demands the full list of objects before confirmation. Only a self-computed set can provide
  it. An un-previewable prune could not be offered at all under FR-027.
- FR-025 demands *excluding* objects that belong to unadopted projects. A broad prune cannot
  exclude anything — it takes what it takes. Targeted removal by identity can.
- FR-023's post-verification becomes meaningful: compare the computed set against a re-read
  inventory and report the difference, which also catches the race in the spec's edge case where
  another process builds mid-prune.

**Consequence**: "Prune" in the UI is a *selection* the operator reviews, not a verb handed
straight to the runtime. Removal is then one targeted invocation per class, with explicit ids.

**Alternative rejected**: issue the broad prune and report what disappeared. Simpler, and
unacceptable — it cannot enumerate in advance, cannot exclude, and on this machine would reach
projects Fathom does not own.

---

## D4. Live resource usage without becoming the load

**Decision**: a discrete `docker stats --no-stream --format '{{json .}}' <explicit ids>` tick,
scoped to the containers currently visible, run only while the usage view is shown.

**Verified**: `--no-stream --format '{{json .}}'` returns one object per container with `CPUPerc`
("0.00%"), `MemPerc`, `MemUsage` ("24.56MiB / 31.25GiB"), `BlockIO`, `NetIO`, `ID` and `Container`.
Figures arrive as pre-formatted strings, so the backend parses them into numbers once and the
protocol carries numbers, not display strings.

**Rationale**: `docker stats` without `--no-stream` is a long-lived process emitting on an
interval — a background sampler that would keep running behind a hidden view and violate FR-045.
A discrete tick is startable and stoppable with the view's visibility, which is exactly what
Principle V asks for. Passing explicit ids rather than letting it sample every container matters on
this machine: an unscoped call samples unrelated projects' containers too, which costs CPU for data
nobody is looking at.

**Cost acknowledged**: each tick is a process spawn and the daemon samples cgroups to answer it.
Mitigation: minimum interval of 2 s, scoped to visible rows, zero ticks when the view is hidden,
and the bounded history from 036 rather than unbounded accumulation.

---

## D5. Logs and interactive sessions

**Decision**: adapt, do not rebuild. `logs --follow --timestamps` as a supervised child process
feeding the existing `src/backend/log_streams.rs` path; `exec -it` on a PTY through the existing
`src/backend/node_shell.rs` machinery.

**Rationale**: bounded buffering, explicit truncation marking, search, batching and clean session
teardown are all solved there for pod logs and node shells (002), and FR-035 explicitly requires
"the same" behaviour. Reimplementing it would mean a second set of truncation semantics to keep
honest.

**Two differences to handle**: a stopped container's `logs` exits immediately rather than
streaming, which FR-035's "retained output, no stream left open" needs; and `exec` must be gated on
adoption per FR-013, because a shell inside a container is unbounded execution — the one
*non*-mutating capability that adoption still withholds.

---

## D6. Adoption: storage, and failing closed

**Decision**: a JSON document at `~/.fathom/container_adoption.json`, written through
`src/backend/persistence.rs::fathom_config_dir()` beside `integrations.json`. It holds adopted
project names, each with when it was adopted. Keyed by project name, never by container id.

**Rationale**: FR-015 requires adoption to be per project, to persist across restarts, and to
cover containers that join later — all three follow from keying on the name the containers
themselves declare. The existing persistence helper already owns directory creation and the
config-directory convention.

**Failing closed is the whole point** (FR-047): an unreadable or malformed record yields *no*
adopted projects. The fallback must never be "everything adopted", because the failure mode of a
corrupt file would then be a machine-wide destructive surface. Principle VI's "fall back without
deleting the file" applies unchanged — a corrupt record is read, rejected, reported, and left on
disk.

**Alternative rejected**: a field inside the existing cluster settings store. Adoption is not
cluster-scoped; a container runtime has no context name to hang it on, which is the same reason
FR-005 keeps this out of the fleet model.

---

## D7. Keeping idle cost at zero

**Decision**: no invocation of any kind while the Docker scene is not shown. Presence comes from
the cached prerequisites the existing kind probe already produces; inventory refresh is scoped to
the visible scene and to explicit operator refreshes; usage sampling is scoped to the visible usage
view (D4).

**Verified precedent**: `kind::detect_prerequisites` already wraps its probe in
`tokio::time::timeout` with `spawn_blocking`, so a hung runtime cannot block startup — FR-001's
timeout requirement is already satisfied by reuse rather than new code.

---

## D8. Project definition files, read-only

**Decision**: discover a project's definition file from the label the project's own tooling writes
(`com.docker.compose.project.config_files`), verified present in the live label set. Read it only to
name it in a preview and to decide whether project-level operations are available.

**Rationale**: FR-033 and Principle VI. The file is the operator's. It is never written, moved or
deleted, and an unreadable or missing one disables project-level operations for that project with
the reason stated, rather than being regenerated from the running containers — which would look
helpful and would silently replace the operator's own file with Fathom's idea of it.

---

## D9. Cluster cross-link

**Decision**: read the cluster-node label the local-cluster tooling writes
(`io.x-k8s.kind.cluster`) and treat it as the sole source of cluster identity, per FR-042.

**Rationale**: it is authoritative and already present on nodes of clusters Fathom provisions.
Naming patterns are explicitly not used: a container called `something-control-plane` that carries
no label is not a cluster node, and inferring otherwise would make the FR-041 break-warning fire on
the wrong object — or worse, not fire on the right one.

**FR-016 interaction**: because cluster nodes carry no *project* label, they fall in the
no-project bucket and would be unadopted by default, making clusters Fathom itself created
unactionable. FR-016 resolves this: a container whose cluster is one Fathom manages is actionable
without a separate adoption step, and the adoption gate consults the cluster link, not just the
project name.

---

## D10. Hosted mode

**Decision**: add `HostedDenial::ContainerRuntime` rather than reuse `HostedDenial::Kind`.

**Rationale**: FR-043 requires the denial to state *why*. `HostedDenial::Kind` is documented as
"Local kind cluster provisioning"; this feature's reason is different — a server has no local
container runtime, and exposing the host's daemon to signed-in users would hand them privileges
they do not have. The policy table in `src/protocol/v1/mutation.rs` is deliberately exhaustive with
no wildcard, so every new command variant must be classified there and the compiler enforces it.

---

## D11. Test fixtures, sized for a shared machine

**Decision**: one image, `busybox` (~4 MB). Compose fixture is two `busybox` services running
`sleep`. Fixture names carry a recognisable prefix; teardown removes by name.

**Rationale**: the operator's machine runs their own work and other agents. `busybox` provides a
shell, so it covers the session tests and `alpine` is unnecessary. Almost no test needs a daemon at
all — the mock `Runner` covers inventory, grouping, preview, blast radius, adoption and the
mutation gate, and the scripted fake backend covers the UI.

**Hard rule for this feature's own suite**: never prune, never remove an image, never stop a
container the test did not create. Verified reason, not a hypothetical: the live daemon currently
runs `thunderforge-app` (five containers) and `slam-usa` alongside anything Fathom does. A suite
that pruned would destroy another project's work, which is the same failure the feature exists to
prevent.

---

## Open questions

None. Both of the spec's `[NEEDS CLARIFICATION]` markers were answered before planning began
(form-driven creation only; read-only unless adopted) and are recorded in the spec's Clarifications
section.
