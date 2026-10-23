/**
 * The Role/ClusterRole rules editor (042-role-create): one row per `rules[]` entry with API
 * groups, resources, verb checkboxes (plus free-text extra verbs) and optional resource names,
 * and a preset switch (read-only / edit / custom) that rewrites every row's verbs.
 *
 * API group and resource suggestions come from discovery (`RefreshApiResources`); free text is
 * always accepted so a CRD or a wildcard can be named even when discovery did not list it.
 */
import { Button } from '@companion/ui'
import type { ApiResourceCatalog } from '@/protocol/types'

import { TokenInput } from './TokenInput'
import {
  PRESET_LABELS,
  STANDARD_VERBS,
  applyPreset,
  emptyRule,
  groupOptions,
  presetOf,
  resourceOptions,
  ruleProblems,
  toggleVerb,
  type PresetId,
  type RoleKind,
  type RuleInput,
} from './roleYaml'

const SELECT_CLASS =
  'h-8 min-w-0 rounded-none border border-input bg-transparent px-2.5 py-1 text-xs outline-none focus-visible:border-ring focus-visible:ring-1 focus-visible:ring-ring dark:bg-input/30'

export interface RuleEditorProps {
  kind: RoleKind
  rules: readonly RuleInput[]
  onChange: (next: RuleInput[]) => void
  catalog: ApiResourceCatalog | undefined
}

export function RuleEditor({ kind, rules, onChange, catalog }: RuleEditorProps) {
  const preset = presetOf(rules)
  const groups = groupOptions(catalog)
  const update = (i: number, next: RuleInput) =>
    onChange(rules.map((r, idx) => (idx === i ? next : r)))

  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-end justify-between gap-2 text-xs">
        <span className="font-medium">Rules</span>
        <label htmlFor="rbac-role-preset-select" className="flex items-center gap-2">
          Verbs preset
          <select
            id="rbac-role-preset-select"
            data-testid="rbac-role-preset"
            className={SELECT_CLASS}
            value={preset}
            onChange={(e) => onChange(applyPreset(rules, e.target.value as PresetId))}
          >
            {(Object.keys(PRESET_LABELS) as PresetId[]).map((p) => (
              <option key={p} value={p}>
                {PRESET_LABELS[p]}
              </option>
            ))}
          </select>
        </label>
      </div>

      {rules.map((rule, i) => {
        const problems = ruleProblems(rule, kind)
        const extraVerbs = rule.verbs.filter(
          (v) => !(STANDARD_VERBS as readonly string[]).includes(v),
        )
        return (
          <div
            key={i}
            data-testid="rbac-rule-row"
            className="flex flex-col gap-2 border border-border/60 p-2"
          >
            <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
              <TokenInput
                label="API groups"
                testId="rbac-rule-api-groups"
                coreGroup
                placeholder="core, apps, *"
                values={rule.apiGroups}
                suggestions={groups}
                onChange={(apiGroups) => update(i, { ...rule, apiGroups })}
              />
              <TokenInput
                label="Resources"
                testId="rbac-rule-resources"
                placeholder="pods, deployments, *"
                values={rule.resources}
                suggestions={resourceOptions(catalog, rule.apiGroups, kind)}
                onChange={(resources) => update(i, { ...rule, resources })}
              />
            </div>
            <fieldset className="flex flex-col gap-1 text-xs">
              <legend className="mb-1">Verbs</legend>
              <div className="flex flex-wrap gap-x-3 gap-y-1">
                {STANDARD_VERBS.map((verb) => (
                  <label key={verb} className="flex items-center gap-1 font-mono">
                    <input
                      type="checkbox"
                      data-testid={`rbac-rule-verb-${verb}`}
                      className="size-3.5 accent-primary"
                      checked={rule.verbs.includes(verb)}
                      onChange={(e) => update(i, toggleVerb(rule, verb, e.target.checked))}
                    />
                    {verb}
                  </label>
                ))}
              </div>
              <TokenInput
                label="Other verbs (optional)"
                testId="rbac-rule-extra-verbs"
                placeholder="*, bind, escalate, impersonate"
                values={extraVerbs}
                suggestions={['*', 'bind', 'escalate', 'impersonate', 'use', 'approve', 'sign']}
                onChange={(extra) =>
                  update(i, {
                    ...rule,
                    verbs: [
                      ...rule.verbs.filter((v) =>
                        (STANDARD_VERBS as readonly string[]).includes(v),
                      ),
                      ...extra,
                    ],
                  })
                }
              />
            </fieldset>
            <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
              <TokenInput
                label="Resource names (optional)"
                testId="rbac-rule-resource-names"
                placeholder="only these objects"
                values={rule.resourceNames}
                onChange={(resourceNames) => update(i, { ...rule, resourceNames })}
              />
              {kind === 'ClusterRole' && (
                <TokenInput
                  label="Non-resource URLs (optional)"
                  testId="rbac-rule-non-resource-urls"
                  placeholder="/healthz, /metrics"
                  values={rule.nonResourceURLs}
                  suggestions={['/healthz', '/livez', '/readyz', '/metrics', '/version', '/api/*']}
                  onChange={(nonResourceURLs) => update(i, { ...rule, nonResourceURLs })}
                />
              )}
            </div>
            <div className="flex items-start justify-between gap-2">
              <div className="flex flex-col gap-0.5">
                {problems.map((p) => (
                  <p key={p} data-testid="rbac-rule-error" className="text-xs text-destructive">
                    {p}
                  </p>
                ))}
              </div>
              <Button
                type="button"
                data-testid="rbac-rule-remove"
                size="sm"
                variant="ghost"
                onClick={() => onChange(rules.filter((_, idx) => idx !== i))}
              >
                Remove rule
              </Button>
            </div>
          </div>
        )
      })}

      <Button
        type="button"
        data-testid="rbac-rule-add"
        size="sm"
        variant="outline"
        className="w-fit"
        onClick={() => onChange([...rules, emptyRule(preset === 'custom' ? 'read-only' : preset)])}
      >
        Add rule
      </Button>
      {rules.length === 0 && (
        <p data-testid="rbac-rule-error" className="text-xs text-destructive">
          At least one rule is required.
        </p>
      )}
      {catalog && catalog.failed_groups.length > 0 && (
        <p data-testid="rbac-rule-discovery-partial" className="text-xs text-muted-foreground">
          Discovery failed for {catalog.failed_groups.join(', ')}; type those resources by hand.
        </p>
      )}
    </div>
  )
}
