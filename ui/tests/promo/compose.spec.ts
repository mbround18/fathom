/**
 * Promo compose: turns the shots captured by capture.spec.ts into a branded HTML brochure and a
 * PDF, using Playwright's own Chromium (already a devDependency — no new tool to keep working).
 * Runs after capture.spec.ts (Playwright orders spec files alphabetically within testDir).
 *
 * Facts (pod/workload counts) are read live from the cluster, same as the screenshots, so the
 * brochure never drifts from what was actually photographed. Output: promo-out/facts.json,
 * promo-out/brochure.html, promo-out/fathom-promo.pdf.
 */
import { mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'

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
}

/** Narrative order: wide feature shots first, then ultra, then down to phone. Matches the
 * insertion order of SHOT_META, not filename order. */
function loadShots(): Shot[] {
  const present = new Set(readdirSync(SHOTS_DIR).filter((f) => f.endsWith('.png')))
  const shots = Object.entries(SHOT_META)
    .filter(([name]) => present.has(`${name}.png`))
    .map(([name, meta]) => {
      const bytes = readFileSync(`${SHOTS_DIR}${name}.png`)
      return {
        name,
        title: meta.title,
        caption: meta.caption,
        dataUrl: `data:image/png;base64,${bytes.toString('base64')}`,
      } satisfies Shot
    })
  const missing = [...present].filter((f) => !(f.replace(/\.png$/, '') in SHOT_META))
  if (missing.length > 0) {
    throw new Error(`promo/compose: no SHOT_META entry for shot(s): ${missing.join(', ')}`)
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
// - Lens/OpenLens, Headlamp: Electron-based, not native desktop apps.
// - None of the compared tools ship a built-in GitOps (Flux) reconciliation view.
// - None of the compared tools ship a built-in eBPF/Cilium/Hubble flow view.
// - K9s has no GUI at all (terminal-only), so several axes are simply not applicable.
const COMPARISON_COLUMNS = ['Lens / OpenLens', 'Headlamp', 'K9s', 'Rancher Dashboard', 'K8s Dashboard']
const COMPARISON_ROWS: ComparisonRow[] = [
  { axis: 'Native desktop app', fathom: 'yes', cells: ['no', 'no', 'yes', 'no', 'no'] },
  { axis: 'GitOps (Flux) view built in', fathom: 'yes', cells: ['no', 'no', 'no', 'no', 'no'] },
  {
    axis: 'eBPF / Cilium traffic view built in',
    fathom: 'yes',
    cells: ['no', 'no', 'no', 'no', 'no'],
  },
  { axis: 'Guarded bulk actions', fathom: 'yes', cells: ['partial', 'partial', 'no', 'partial', 'no'] },
  { axis: 'Free / OSS', fathom: 'yes', cells: ['partial', 'yes', 'yes', 'yes', 'yes'] },
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

/** Small header/footer bar baked into every page: brand + internal label, date + page number. */
function pageChrome(cls: string, inner: string, pageNum: number, totalPages: number, facts: Facts): string {
  return `
<div class="page ${cls}">
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
  <p class="comparison-note">✓ built in &middot; ~ partial or bolt-on &middot; &mdash; not offered. Lens/OpenLens and Headlamp are Electron-based, not native desktop apps; K9s is terminal-only (no GUI). None of the tools above ship a built-in GitOps reconciliation view or a built-in eBPF/Cilium flow view.</p>`

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
