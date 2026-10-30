# UI contract: Keel guided UI

These are the stable surfaces that tests and other features rely on. Everything here is client-side.

## Routes

| Route | Page | Notes |
|---|---|---|
| `/help` | `help/HelpPage.tsx` | Renders `docs/guides/keel/README.md` as the index. Lazy chunk. |
| `/help/:guide` | `help/HelpPage.tsx` | An unknown `:guide` shows the existing NotFound page. The hash scrolls to the matching heading. |

The account menu gains **Help**, which goes to `/help`. It appears after Passkeys, with
`QuestionIcon`.

## Components

```ts
// ui/EmptyState.tsx (existing props unchanged)
EmptyState(props: {
  icon?: ReactNode
  title: string
  reason?: ReactNode
  steps?: ReactNode[]          // 2–3
  action?: ReactNode           // the one primary button
  whoCan?: string              // used only when `action` is absent
  verb?: string
  guide?: GuideRef
  children?: ReactNode
})

// ui/empty-variant.ts
emptyVariant(i: { filtered: boolean; canAct: boolean }): 'no-match' | 'guided' | 'restricted'

// ui/NoMatch.tsx
NoMatch(props: { what: string; onClear: () => void })   // "No {what} match." + Clear filter

// packages/ui/src/info-tip.tsx (exported from @companion/ui; generic)
InfoTip(props: {
  label: string                 // the term; the trigger reads "About {label}"
  children: ReactNode           // the explanation
  footer?: ReactNode            // e.g. a Learn more link
  className?: string
  contentData?: Record<`data-${string}`, string>  // put on the popover content
} & Record<`data-${string}`, string>)  // passed to the trigger

// help/TermTip.tsx (Keel)
TermTip(props: { term: TermId })   // GLOSSARY[term] → InfoTip, footer = GuideLink

// ui/GuideLink.tsx
GuideLink(props: { to: GuideRef; children?: ReactNode })  // default text "Learn more"

// ui/PageHeader.tsx (adds `guide`)
PageHeader(props: { …existing; guide?: GuideRef })

// help/guides.ts
type GuideSlug = 'getting-started' | 'hosts' | 'receivers' | 'bindings' | 'reviews' | 'inventory'
  | 'enrollment-template' | 'oidc-roles' | 'system-workspace' | 'updates'
interface GuideRef { guide: GuideSlug; anchor?: string }
guideHref(ref: GuideRef): string
loadGuide(slug: GuideSlug | 'README'): Promise<string>

// app/nav.ts
NAV_GROUPS: readonly NavGroup[]
visibleGroups(me: Me | null): NavGroup[]
SECTIONS: readonly Section[]   // = NAV_GROUPS.flatMap(g => g.sections)
```

## Behaviour

- **EmptyState**:
  - It shows `action` or `whoCan`, never both, and never a disabled button (FR-012).
  - The steps render as an `<ol>` with visible numbers, and the block is centred in the content
    area.
  - The guide link sits under the action.
- **InfoTip**:
  - It opens on mouse hover, keyboard focus and click or tap, and closes on Escape, blur, a
    pointer-down outside it, and mouse leave (unless it was opened by click).
  - Focus stays on the trigger.
  - While it is open, the trigger has `aria-describedby` set to the content id. The trigger's
    `aria-label` is "About {term}" (FR-022).
- **Nav**:
  - It keeps `<nav aria-label="Sections">`, with one `role="group"` per visible group, labelled by
    its heading. The heading is visible from `md` and `sr-only` below.
  - Below `md`, each group after the first has a left divider.
  - The current link keeps `aria-current="page"`.
  - Headings use `text-muted-foreground`, never the accent (FR-002, FR-006).

## Test hooks (`data-*`)

| Attribute | On | Values |
|---|---|---|
| `data-nav-group` | each group wrapper | `pipelines`, `infrastructure`, `administration` |
| `data-pane-state="empty"` | EmptyState root | existing |
| `data-empty` | EmptyState or NoMatch root | `guided`, `restricted`, `no-match` |
| `data-empty-page` | EmptyState root | `runs`, `reviews`, `bindings`, `receivers`, `inventory`, `hosts`, `workspaces`, `teams`, `workspace-picker` |
| `data-empty-action` | the action wrapper | present when an action is shown |
| `data-who-can` | the restricted line | present when restricted |
| `data-guide-link` | GuideLink `<a>` | the guide slug |
| `data-infotip` | InfoTip trigger | the `TermId` |
| `data-infotip-content` | popover content | the `TermId` |
| `data-copy-block` | CopyBlock root | existing; new values: `receiver-url`, `receiver-secret`, `enroll-token`, `enroll-command` (keep existing names where they already exist) |
| `data-help-guide` | HelpPage article | the slug, or `README` |

Shared-package hooks: the InfoTip trigger has `data-slot="info-tip-trigger"` and the content
`data-slot="info-tip-content"`. Keel's `TermTip` adds `data-infotip` and `data-infotip-content`.
