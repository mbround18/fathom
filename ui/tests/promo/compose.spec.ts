/**
 * Promo compose: turns the shots captured by capture.spec.ts into a branded HTML brochure and a
 * PDF, using Playwright's own Chromium (already a devDependency — no new tool to keep working).
 * Runs after capture.spec.ts (Playwright orders spec files alphabetically within testDir).
 *
 * Facts (pod/workload counts) are read live from the cluster, same as the screenshots, so the
 * brochure never drifts from what was actually photographed. Output: promo-out/facts.json,
 * promo-out/brochure.html, promo-out/fathom-promo.pdf.
 */
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

import { BRAND_GLYPH, FONT_FACE_CSS, PALETTE, PALETTE_LIGHT } from './brand'
import {
  expect,
  kubectlJson,
  LIVE_NAMESPACE,
  liveContext,
  OUT_DIR,
  redactions,
  SHOTS_DIR,
  test,
  type KubePod,
} from './helpers'

/** Checked-in static promo assets (e.g. the staged Terminal screenshot) — not written by
 * capture.spec.ts. See research.md R6: the Terminal tab is never driven by the automated
 * pipeline, so its showcase image lives here as a hand-captured PNG instead. */
const ASSETS_DIR = fileURLToPath(new URL('./assets/', import.meta.url))

/** SHOT_META keys sourced from ui/tests/promo/assets/ (staged) instead of promo-out/shots/
 * (captured live by capture.spec.ts this run). */
const STAGED_SHOTS = new Set(['terminal-shot'])

interface WorkloadCounts {
  kind: string
  ready: number
  total: number
}

interface Facts {
  generatedAt: string
  namespace: string
  podCount: number
  podsRunning: number
  nodeCount: number
  workloads: WorkloadCounts[]
}

function countWorkloads(kind: string, plural: string): WorkloadCounts {
  const list = kubectlJson<{
    items: { status?: { replicas?: number; readyReplicas?: number } }[]
  }>(['get', plural])
  const total = list.items.length
  const ready = list.items.filter(
    (i) => (i.status?.readyReplicas ?? 0) >= (i.status?.replicas ?? 1),
  ).length
  return { kind, ready, total }
}

function gatherFacts(): Facts {
  const pods = kubectlJson<{ items: KubePod[] }>(['get', 'pods']).items
  const running = pods.filter((p) => p.status?.phase === 'Running').length
  const nodeCount = new Set(pods.map((p) => p.spec?.nodeName).filter(Boolean)).size
  return {
    generatedAt: new Date().toISOString(),
    namespace: LIVE_NAMESPACE,
    podCount: pods.length,
    podsRunning: running,
    nodeCount,
    workloads: [
      countWorkloads('Deployments', 'deployments'),
      countWorkloads('StatefulSets', 'statefulsets'),
      countWorkloads('DaemonSets', 'daemonsets'),
    ],
  }
}

interface Shot {
  name: string
  title: string
  caption: string
  dataUrl: string
}

const SHOT_META: Record<string, { title: string; caption: string; scale: number }> = {
  'wide-inspector': {
    title: 'Inspect at a glance',
    caption: 'Details, YAML, and events for any resource — one click away, on a 1920px desktop.',
    scale: 2,
  },
  'wide-scale': {
    title: 'Scale with confidence',
    caption: 'Every action previews its effect before it ships — no surprise replica counts.',
    scale: 2,
  },
  'wide-confirm': {
    title: 'Bulk actions, guarded',
    caption: 'Multi-select deletes list every target by name before anything is sent.',
    scale: 2,
  },
  ultra: {
    title: 'Built for the biggest screens',
    caption: 'Six independent slots on an ultra-wide monitor — nothing crowds the data.',
    scale: 1,
  },
  tablet: {
    title: 'Tablet-ready',
    caption: 'The full picture, comfortably tapped.',
    scale: 2,
  },
  'phone-list': {
    title: 'Fits in your pocket',
    caption: '44px touch targets and a searchable list, down to a 390px phone.',
    scale: 3,
  },
  'phone-inspector': {
    title: 'Full detail, small screen',
    caption: 'The same inspector as desktop, stacked for one-thumb reading.',
    scale: 3,
  },
  'phone-nav': {
    title: 'Every resource, one tap away',
    caption: 'A drawer navigator keeps the whole cluster reachable on mobile.',
    scale: 3,
  },
  'wide-flux': {
    title: 'GitOps, built in',
    caption:
      'The Flux panel reads GitRepositories, Kustomizations, and HelmReleases straight from the cluster and shows exactly what is ready, reconciling, or failed.',
    scale: 2,
  },
  'wide-hubble': {
    title: 'eBPF traffic, live',
    caption:
      'The Hubble panel talks directly to Cilium’s Hubble Relay over gRPC and shows real forwarded and dropped flow traffic between namespaces — no separate dashboard required.',
    scale: 2,
  },
  'wide-logs': {
    title: 'Tail logs without leaving the app',
    caption:
      'The Logs tab live-streams real container output straight from the cluster, with auto-scroll and a jump-to-latest control.',
    scale: 2,
  },
  'wide-applications-list': {
    title: 'Applications, not just objects',
    caption:
      'Helm releases and Flux Kustomizations are grouped into one row each — Instance, Application, Namespace, Managed By, Version, Age, and Status, rolled up from every workload they own.',
    scale: 2,
  },
  'wide-applications-inspector': {
    title: 'One app, every resource it owns',
    caption:
      'The inspector rolls up an application’s metadata, aggregate CPU/memory, and every Deployment, StatefulSet, and Pod it owns — each one a click away.',
    scale: 2,
  },
  'wide-applications-drilldown': {
    title: 'Follow the relationship, keep your place',
    caption:
      'Click "owns" on a related Pod and the inspector drills straight in — a breadcrumb tracks the path back to the application, so context is never lost.',
    scale: 2,
  },
  'wide-helm-list': {
    title: 'Helm, read straight from the source',
    caption:
      'Every installed release — name, chart, chart version, app version, revision, and status — read directly from Helm’s own release storage, no `helm` binary required.',
    scale: 2,
  },
  'wide-helm-inspector': {
    title: 'Every release, fully unpacked',
    caption:
      'User-supplied and effective values, the rendered manifest, and full revision history for one release — the same install/upgrade history `helm history` shows, in the app.',
    scale: 2,
  },
  'wide-helm-origin-link': {
    title: 'Trace a release back to its origin',
    caption:
      'A Flux-managed release links straight to the HelmRelease that owns it, with its live Ready state and reconciliation message — no more guessing why GitOps hasn’t caught up.',
    scale: 2,
  },
  'terminal-shot': {
    title: 'A real shell, inside the app',
    caption:
      'The Terminal tab opens a genuine interactive `kube exec` session in the selected container — resizable, ANSI-aware, no separate window.',
    scale: 2,
  },
}

/** Narrative order: wide feature shots first, then ultra, then down to phone. Matches the
 * insertion order of SHOT_META, not filename order. */
function loadShots(): Shot[] {
  const present = new Set(readdirSync(SHOTS_DIR).filter((f) => f.endsWith('.png')))
  const missing = [...present].filter(
    (f) => !(f.replace(/\.png$/, '') in SHOT_META) || STAGED_SHOTS.has(f.replace(/\.png$/, '')),
  )
  if (missing.length > 0) {
    throw new Error(`promo/compose: no SHOT_META entry for shot(s): ${missing.join(', ')}`)
  }
  // Narrative order follows SHOT_META's insertion order (not filename/readdir order), so a
  // staged entry (e.g. Terminal) can sit right next to the live entry it belongs beside (Logs).
  const shots: Shot[] = []
  for (const [name, meta] of Object.entries(SHOT_META)) {
    const staged = STAGED_SHOTS.has(name)
    const dir = staged ? ASSETS_DIR : SHOTS_DIR
    const path = `${dir}${name}.png`
    if (staged) {
      if (!existsSync(path)) {
        throw new Error(
          `promo/compose: staged asset missing: ${path}\n` +
            'Capture it by hand per research.md R6 and save it there (never via capture.spec.ts).',
        )
      }
    } else if (!present.has(`${name}.png`)) {
      continue
    }
    const bytes = readFileSync(path)
    shots.push({
      name,
      title: meta.title,
      caption: meta.caption,
      dataUrl: `data:image/png;base64,${bytes.toString('base64')}`,
    })
  }
  return shots
}

const HTML_ESCAPES: Record<string, string> = {
  '&': '&amp;',
  '<': '&lt;',
  '>': '&gt;',
  '"': '&quot;',
  "'": '&#39;',
}

function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => HTML_ESCAPES[c] ?? c)
}

function statTile(label: string, value: string): string {
  return `<div class="stat"><div class="stat-value">${escapeHtml(value)}</div><div class="stat-label">${escapeHtml(label)}</div></div>`
}

type Cell = 'yes' | 'no' | 'partial'

interface ComparisonRow {
  axis: string
  fathom: Cell
  cells: Cell[]
}

// Modest, accurate claims only: no fabricated competitor features.
// - Lens (Mirantis): Electron-based, not a native desktop app; free tier exists but Team/Enterprise
//   tiers are paid subscriptions ($9.99/user/mo and up) — mirantis.com/blog/lens-plans-pricing-update.
// - OpenLens: the community-maintained free fork of pre-acquisition Lens; same Electron base.
// - Headlamp: CNCF Sandbox project, Electron-based, fully free/OSS, built around a plugin system —
//   a community Flux CD plugin exists (oneuptime.com/blog/2026-03-06-use-headlamp-flux-cd), so its
//   GitOps view is "partial" (opt-in plugin), not built in by default. No Cilium/Hubble plugin exists.
// - None of the compared tools ship a built-in eBPF/Cilium/Hubble flow view.
// - K9s has no GUI at all (terminal-only), so several axes are simply not applicable.
const COMPARISON_COLUMNS = ['Lens', 'OpenLens', 'Headlamp', 'K9s', 'Rancher Dashboard', 'K8s Dashboard']
const COMPARISON_ROWS: ComparisonRow[] = [
  { axis: 'Native desktop app', fathom: 'yes', cells: ['no', 'no', 'no', 'yes', 'no', 'no'] },
  {
    axis: 'GitOps (Flux) view built in',
    fathom: 'yes',
    cells: ['no', 'no', 'partial', 'no', 'no', 'no'],
  },
  {
    axis: 'eBPF / Cilium traffic view built in',
    fathom: 'yes',
    cells: ['no', 'no', 'no', 'no', 'no', 'no'],
  },
  {
    axis: 'Extension / plugin ecosystem',
    fathom: 'no',
    cells: ['yes', 'yes', 'yes', 'no', 'partial', 'no'],
  },
  {
    axis: 'Guarded bulk actions',
    fathom: 'yes',
    cells: ['partial', 'partial', 'partial', 'no', 'partial', 'no'],
  },
  { axis: 'Free / OSS, no paid tier', fathom: 'yes', cells: ['no', 'yes', 'yes', 'yes', 'yes', 'yes'] },
]

const CELL_GLYPH: Record<Cell, string> = { yes: '✓', no: '—', partial: '~' }

function comparisonCell(c: Cell, emphasize: boolean): string {
  const cls = `cell cell-${c}${emphasize ? ' cell-fathom' : ''}`
  return `<td class="${cls}">${CELL_GLYPH[c]}</td>`
}

function comparisonTable(): string {
  const header = [
    '<th class="row-axis">Capability</th>',
    '<th class="col-fathom">Fathom</th>',
    ...COMPARISON_COLUMNS.map((c) => `<th>${escapeHtml(c)}</th>`),
  ].join('')
  const rows = COMPARISON_ROWS.map(
    (r) =>
      `<tr><td class="row-axis">${escapeHtml(r.axis)}</td>${comparisonCell(r.fathom, true)}${r.cells
        .map((c) => comparisonCell(c, false))
        .join('')}</tr>`,
  ).join('\n')
  return `<table class="comparison"><thead><tr>${header}</tr></thead><tbody>${rows}</tbody></table>`
}

/** A small, evenly-tiled diagonal watermark, the way "CONFIDENTIAL"/"DRAFT" stamps look on real
 *  legal/financial PDFs: quiet and repeated, not one oversized word punched through the content. */
function watermarkTileDataUri(color: string): string {
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="360" height="220">` +
    `<g transform="rotate(-28 180 110)">` +
    `<text x="-20" y="118" font-family="Arial, Helvetica, sans-serif" font-size="15" ` +
    `font-weight="600" letter-spacing="3" fill="${color}" fill-opacity="0.55">INTERNAL EVALUATION</text>` +
    `</g></svg>`
  return `data:image/svg+xml,${encodeURIComponent(svg)}`
}
const WATERMARK_TILE_LIGHT = watermarkTileDataUri(PALETTE_LIGHT.mutedForeground)
const WATERMARK_TILE_DARK = watermarkTileDataUri(PALETTE.mutedForeground)

/** Small header/footer bar baked into every page: brand + internal label, date + page number. */
function pageChrome(cls: string, inner: string, pageNum: number, totalPages: number, facts: Facts): string {
  return `
<div class="page ${cls}">
  <div class="page-watermark" aria-hidden="true"></div>
  <div class="page-header">
    <span class="page-brand">${BRAND_GLYPH} FATHOM</span>
    <span class="page-tag">Internal &middot; Confidential</span>
  </div>
  ${inner}
  <div class="page-footer">
    <span>Generated ${escapeHtml(new Date(facts.generatedAt).toUTCString())}</span>
    <span>Page ${String(pageNum)} of ${String(totalPages)}</span>
  </div>
</div>`
}

function buildHtml(facts: Facts, shots: Shot[]): string {
  const nodeNote =
    redactions().length > 0
      ? '<p class="node-note">Node identifiers shown as node-1, node-2, … (redacted).</p>'
      : ''
  const stats = [
    statTile('Pods watched live', String(facts.podCount)),
    statTile('Running', `${String(facts.podsRunning)}/${String(facts.podCount)}`),
    statTile('Cluster nodes', String(facts.nodeCount)),
    ...facts.workloads.map((w) => statTile(w.kind, `${String(w.ready)}/${String(w.total)} ready`)),
  ].join('\n')

  const shotBodies = shots.map(
    (s, i) => `
    <section class="shot ${i === 0 ? 'shot-hero' : ''}">
      <div class="shot-text">
        <h2>${escapeHtml(s.title)}</h2>
        <p>${escapeHtml(s.caption)}</p>
      </div>
      <div class="shot-frame"><img src="${s.dataUrl}" alt="${escapeHtml(s.title)}" /></div>
    </section>`,
  )

  const coverBody = `
  <div class="brand-glyph">${BRAND_GLYPH}</div>
  <div class="wordmark">FATHOM</div>
  <div class="tagline">Browse, inspect, and act on your Kubernetes cluster</div>
  <div class="meta">Captured live from the fathom-dev sandbox &middot; ${escapeHtml(new Date(facts.generatedAt).toUTCString())}</div>`

  const statsBody = `
  <div class="section-label">Live from the cluster</div>
  <h1>What Fathom is watching right now</h1>
  <p>Every number below was read from a real Kubernetes namespace at generation time &mdash; not staged data.</p>
  <div class="stats">${stats}</div>
  ${nodeNote}`

  const comparisonBody = `
  <div class="section-label">How Fathom compares</div>
  <h1>One native app, not five browser tabs</h1>
  <p>Feature comparison against the other tools teams reach for when kubectl isn't enough.</p>
  ${comparisonTable()}
  <p class="comparison-note">✓ built in &middot; ~ partial or bolt-on &middot; &mdash; not offered. Lens, OpenLens, and Headlamp are Electron-based, not native desktop apps; K9s is terminal-only (no GUI). Lens is free for individuals but its Team ($9.99/user/mo) and Enterprise tiers are paid; OpenLens is the free community fork. Headlamp's Flux view comes from an optional community plugin, not the default build &mdash; and like the rest, it has no Cilium/Hubble plugin. No tool above ships a built-in eBPF/Cilium flow view.</p>`

  const totalPages = 3 + shotBodies.length
  const pages = [
    pageChrome('cover', coverBody, 1, totalPages, facts),
    pageChrome('', statsBody, 2, totalPages, facts),
    pageChrome('', comparisonBody, 3, totalPages, facts),
    ...shotBodies.map((body, i) => pageChrome('', body, 4 + i, totalPages, facts)),
  ].join('\n')

  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<title>Fathom</title>
<style>
${FONT_FACE_CSS}
:root {
  --bg: ${PALETTE.background};
  --card: ${PALETTE.card};
  --fg: ${PALETTE.foreground};
  --muted: ${PALETTE.mutedForeground};
  --neon: ${PALETTE.accentNeon};
  --magenta: ${PALETTE.accent2};
  --ok: ${PALETTE.statusOk};
  --warn: ${PALETTE.statusWarn};
  --err: ${PALETTE.statusErr};
  --border: ${PALETTE.border};
}
* { box-sizing: border-box; }
body {
  margin: 0;
  background: ${PALETTE_LIGHT.background};
  color: var(--fg);
  font-family: 'JetBrains Mono', ui-monospace, monospace;
  font-size: 14px;
}
.page {
  width: 1120px;
  min-height: 800px;
  padding: 64px 72px;
  page-break-after: always;
  position: relative;
  background: var(--bg);
  color: var(--fg);
}
.page:last-child { page-break-after: auto; }
/* Content pages use the real light "blueprint" theme (tokens.css :root) so the dark
   neon-noir screenshots pop; only the cover keeps the dark hero treatment. */
.page:not(.cover) {
  --bg: ${PALETTE_LIGHT.background};
  --card: ${PALETTE_LIGHT.card};
  --fg: ${PALETTE_LIGHT.foreground};
  --muted: ${PALETTE_LIGHT.mutedForeground};
  --neon: ${PALETTE_LIGHT.accentNeon};
  --magenta: ${PALETTE_LIGHT.accent2};
  --ok: ${PALETTE_LIGHT.statusOk};
  --warn: ${PALETTE_LIGHT.statusWarn};
  --err: ${PALETTE_LIGHT.statusErr};
  --border: ${PALETTE_LIGHT.border};
}
.cover {
  display: flex;
  flex-direction: column;
  justify-content: center;
  align-items: center;
  text-align: center;
  min-height: 900px;
  background:
    radial-gradient(circle at 30% 20%, color-mix(in srgb, var(--neon) 14%, transparent), transparent 55%),
    radial-gradient(circle at 75% 75%, color-mix(in srgb, var(--magenta) 10%, transparent), transparent 55%),
    var(--bg);
}
.brand-glyph {
  font-family: 'Fathom Brand', serif;
  font-size: 120px;
  line-height: 1;
  color: var(--neon);
  text-shadow: 0 0 24px color-mix(in srgb, var(--neon) 70%, transparent);
}
.wordmark {
  font-family: 'Chakra Petch', sans-serif;
  font-weight: 700;
  font-size: 56px;
  letter-spacing: 0.3em;
  margin: 24px 0 8px;
}
.tagline {
  font-family: 'Chakra Petch', sans-serif;
  font-weight: 600;
  font-size: 20px;
  color: var(--muted);
  letter-spacing: 0.05em;
  margin-bottom: 48px;
}
.meta { color: var(--muted); font-size: 13px; }
h1, h2 {
  font-family: 'Chakra Petch', sans-serif;
  font-weight: 700;
  letter-spacing: 0.02em;
}
h1 { font-size: 30px; color: var(--neon); margin: 0 0 8px; }
h2 { font-size: 24px; margin: 0 0 12px; }
p { color: var(--muted); line-height: 1.5; margin: 0; }
.section-label {
  font-family: 'Chakra Petch', sans-serif;
  font-weight: 600;
  letter-spacing: 0.24em;
  text-transform: uppercase;
  font-size: 12px;
  color: var(--neon);
  margin-bottom: 24px;
}
.stats {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 16px;
  margin-top: 32px;
}
.stat {
  border: 1px solid var(--border);
  background: var(--card);
  border-radius: 2px;
  padding: 20px;
}
.stat-value {
  font-family: 'Chakra Petch', sans-serif;
  font-weight: 700;
  font-size: 28px;
  color: var(--ok);
}
.stat-label {
  color: var(--muted);
  font-size: 12px;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  margin-top: 4px;
}
.shot {
  display: flex;
  flex-direction: column;
  gap: 24px;
  height: 672px;
  justify-content: flex-start;
}
.shot-text { max-width: 780px; flex: none; }
.shot-frame {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  border: 1px solid var(--border);
  border-radius: 2px;
  overflow: hidden;
  background: var(--card);
  box-shadow: 0 0 32px color-mix(in srgb, var(--neon) 18%, transparent);
}
.shot-frame img { display: block; max-width: 100%; max-height: 100%; width: auto; height: auto; object-fit: contain; }
.node-note { margin-top: 16px; font-size: 11px; color: var(--muted); }
.page-watermark {
  position: absolute;
  inset: 0;
  pointer-events: none;
  z-index: 5;
  background-image: url("${WATERMARK_TILE_LIGHT}");
  background-repeat: repeat;
  opacity: 0.4;
}
.page.cover .page-watermark {
  background-image: url("${WATERMARK_TILE_DARK}");
  opacity: 0.5;
}
.page-header {
  position: absolute;
  top: 26px;
  left: 72px;
  right: 72px;
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  font-size: 10px;
  letter-spacing: 0.16em;
  text-transform: uppercase;
  color: var(--muted);
}
.page-header .page-brand {
  font-family: 'Chakra Petch', sans-serif;
  font-weight: 700;
  letter-spacing: 0.22em;
  color: var(--neon);
}
.page-header .page-tag {
  color: var(--warn);
  font-weight: 700;
}
.page-footer {
  position: absolute;
  bottom: 26px;
  left: 72px;
  right: 72px;
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  font-size: 10px;
  letter-spacing: 0.04em;
  color: var(--muted);
  border-top: 1px solid var(--border);
  padding-top: 8px;
}
.comparison {
  width: 100%;
  border-collapse: collapse;
  margin-top: 28px;
  font-size: 13px;
}
.comparison th, .comparison td {
  border: 1px solid var(--border);
  padding: 10px 12px;
  text-align: center;
}
.comparison th {
  font-family: 'Chakra Petch', sans-serif;
  font-weight: 600;
  font-size: 12px;
  letter-spacing: 0.04em;
  background: var(--card);
  color: var(--fg);
}
.comparison .row-axis {
  text-align: left;
  font-weight: 600;
  color: var(--fg);
}
.comparison .col-fathom { color: var(--neon); }
.comparison td.cell-fathom {
  background: color-mix(in srgb, var(--neon) 10%, var(--card));
  font-weight: 700;
  color: var(--neon);
}
.comparison td.cell-yes { color: var(--ok); }
.comparison td.cell-no { color: var(--muted); }
.comparison td.cell-partial { color: var(--warn); }
.comparison-note { margin-top: 16px; font-size: 12px; }
</style>
</head>
<body>

${pages}

</body>
</html>
`
}

test.describe('compose', () => {
  test('build facts.json, brochure.html, and fathom-promo.pdf', async ({ page }) => {
    const facts = gatherFacts()
    const shots = loadShots()
    expect(shots.length, 'no shots found; run capture.spec.ts first').toBeGreaterThan(0)

    mkdirSync(OUT_DIR, { recursive: true })
    writeFileSync(`${OUT_DIR}facts.json`, JSON.stringify(facts, null, 2))

    const html = buildHtml(facts, shots)
    writeFileSync(`${OUT_DIR}brochure.html`, html)

    await page.setContent(html, { waitUntil: 'networkidle' })
    await page.evaluate(() => document.fonts.ready)
    await page.pdf({
      path: `${OUT_DIR}fathom-promo.pdf`,
      width: '1120px',
      height: '900px',
      printBackground: true,
      margin: { top: '0', bottom: '0', left: '0', right: '0' },
    })

    console.log(
      `[promo] context '${liveContext()}': wrote ${OUT_DIR}fathom-promo.pdf ` +
        `(${String(shots.length)} shots, ${String(facts.podCount)} pods)`,
    )
  })
})
