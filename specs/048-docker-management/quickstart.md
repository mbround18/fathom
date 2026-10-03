# Phase 1 Quickstart: validating Docker management

**Feature**: [spec.md](./spec.md) | **Plan**: [plan.md](./plan.md) | **Date**: 2026-10-02

How to prove this feature works, in the order that costs the least. Read the resource rules first —
the development machine runs the operator's own work and other agents on one 32-core / 31 GB box,
and the live daemon currently hosts unrelated projects.

---

## Resource rules (not optional)

1. **One image: `busybox`** (~4 MB). Its shell covers the session tests; `alpine` is unnecessary
   and no larger image is to be introduced for this feature's tests.
2. **Never prune. Never remove an image. Never stop a container the test did not create.** The
   daemon hosts other projects' work. Teardown removes fixtures *by name*.
3. **No kind cluster is created for these tests.** The cross-link is verified from mock runner
   output and from the fake backend. Step 5 is only for a human who already has a cluster running.
4. `CARGO_BUILD_JOBS=4`, lower when the machine is busy. Prefer `cargo check` while iterating.
5. Playwright sizes its own workers (`ui/tests/support/workers.ts`); do not pass `--workers`.
   Override with `PLAYWRIGHT_WORKERS` only to pin it lower.

---

## Step 1 — Unit tests, no daemon at all (seconds, free)

Most of the feature is provable here, because the runner is a trait with a mock.

```bash
CARGO_BUILD_JOBS=4 cargo test -p fathom container_runtime
```

Expected to cover, each failing without its implementation:

- all four `RuntimePresenceState` values, including that none of them renders "no containers"
- grouping, with the no-project bucket distinct from a project named anything
- label parsing that does **not** split the ambiguous flat `Labels` string (research D2)
- the adoption gate refusing every mutating command for an unadopted project
- the adoption record failing closed on malformed JSON, and leaving the file on disk
- blast radius naming other projects, and **excluding** unadopted objects with a reason
- `Restarting` surviving as its own state
- unavailable sizes and usage figures staying unavailable rather than becoming `0`
- the argv invariant: issuing executes exactly the previewed argv

## Step 2 — Protocol round-trips and bindings (seconds, free)

```bash
CARGO_BUILD_JOBS=4 cargo test -p fathom --test export_bindings
git diff --stat ui/src/protocol/generated     # must be empty
CARGO_BUILD_JOBS=4 cargo test -p fathom protocol::v1
```

The drift check is what `scripts/ci.sh` enforces; running it here catches it before the gate does.

## Step 3 — UI end-to-end against the scripted fake backend (minutes, no daemon)

```bash
cd ui && pnpm test:e2e -- containers
```

One spec per user story. The fake backend scripts presence, inventory, previews and refusals, so
every acceptance scenario in the spec is reachable without a runtime — including the ones that are
awkward with a real daemon: unreachable sockets, 500 containers, a corrupt definition file, a
preview going stale between confirm and issue.

**When a test needs a request id, poll it — never predict it.** Use
`requestIdOf(page, '<CommandType>')` after the click, not `nextRequestId()` before it. Predicting
races any background command and the rejection then belongs to a request nothing is waiting on.
This has already cost one debugging session on `us4-role-assignment.spec.ts`.

## Step 4 — Live daemon, minimal fixtures (a minute, ~4 MB)

Only this step touches a real runtime. Create the fixtures:

```bash
docker pull busybox                                   # ~4 MB, once
mkdir -p /tmp/fathom-048-fixture && cd /tmp/fathom-048-fixture
cat > compose.yaml <<'YAML'
services:
  one:
    image: busybox
    command: ["sleep", "3600"]
  two:
    image: busybox
    command: ["sleep", "3600"]
YAML
docker compose -p fathom048 up -d
docker run -d --name fathom048-loner busybox sleep 3600    # the no-project bucket
```

Then, with the app running, verify by hand:

1. The Docker section is present, and names the runtime and endpoint.
2. `fathom048` appears as a group with 2 members, 2 running. `fathom048-loner` appears in the
   no-project bucket, not attributed to any project.
3. **Unrelated projects on this daemon are visible and read-only.** Their containers show state,
   ports and logs; no action is offered; the absence says "not adopted" and offers adoption. Do
   **not** adopt them.
4. Adopt `fathom048`. Actions appear. Stop one container: the preview shows the literal argv and
   that one container; the row changes only after the runtime reports.
5. Restart the project: the preview names both members; the outcome reports each separately.
6. Select a prune of dangling images. The preview enumerates each object with its size, names
   every project in the blast radius, and lists any object excluded for belonging to an unadopted
   project. **Cancel it** — this step verifies the preview, not the removal.
7. Follow `fathom048-one`'s logs; confirm batching and an explicit truncation marker. Attempt a
   session on an unadopted container and confirm it is refused with the reason.
8. Open the usage view, confirm per-container and per-project figures; close the scene and confirm
   sampling stops (step 6 of the next section measures this).
9. Release `fathom048`; confirm actions disappear.

Teardown — by name, never by prune:

```bash
docker compose -p fathom048 down
docker rm -f fathom048-loner
rm -rf /tmp/fathom-048-fixture
```

`busybox` is left pulled; removing an image is exactly what rule 2 forbids.

## Step 5 — Cluster cross-link, only if a cluster already exists (human, optional)

Do **not** create a cluster for this. If one happens to be running:

1. Its node containers state which cluster they belong to and link to it.
2. The cluster's own view lists those containers and links back.
3. Previewing a stop on a node container warns that the cluster will break, names it, and says so
   separately when you are currently connected to it. **Cancel.**

Otherwise this is covered by mock runner output carrying the cluster label (Step 1) and by the fake
backend (Step 3).

## Step 6 — Quiet when closed (Principle V, SC-011)

```bash
./scripts/measure-idle-cpu.sh --attach <pid>
```

With the app idle and the Docker scene **closed**, the figure must be indistinguishable from the
pre-feature baseline and under 2% of one core. Stronger check, and the one that actually proves
FR-045: with the scene closed, no runtime process is spawned at all.

```bash
# while the app is idle with the scene closed — expect no new child processes
watch -n 5 'pgrep -a -P <pid> | grep -cE "docker|podman"'
```

## Step 7 — The gate

```bash
./scripts/ci.sh > /tmp/ci-048.log 2>&1; echo "ci_exit=$?"; tail -40 /tmp/ci-048.log
```

Capture the exit code explicitly. Piping to `tail` swallows it — that has already produced one
false green in this repository.

---

## Done when

- Steps 1–3 pass with no daemon involved.
- Step 4's nine checks pass by hand, and teardown leaves the daemon as it was found: other
  projects' containers untouched, nothing pruned.
- Step 6 shows zero runtime invocations with the scene closed.
- `./scripts/ci.sh` exits 0, bindings drift included.
- `./scripts/hygiene.sh` exits 0 with nothing dangling.
