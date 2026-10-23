/**
 * Add/remove Group/User/ServiceAccount subject rows for a RoleBinding/ClusterRoleBinding
 * (011-rbac-oidc-administration US2 T026). Namespace is only meaningful for a `ServiceAccount`
 * subject (FR spec: SA subjects carry `namespace`, User/Group subjects have `namespace: null`).
 *
 * FR-014 client-side validation: an empty subject name is rejected before any YAML is generated
 * — `subjectsValid()` is the pure predicate `RoleBindingForm` uses to gate its Review button, and
 * each offending row shows an inline `data-testid="rbac-subject-error"`.
 */
import { Button, Input } from '@companion/ui'
import type { SubjectSummary } from '@/protocol/types'

const SUBJECT_KINDS = ['User', 'Group', 'ServiceAccount'] as const

export interface SubjectEditorProps {
  subjects: readonly SubjectSummary[]
  onChange: (next: SubjectSummary[]) => void
}

/** FR-014: every subject needs a non-blank name. Exported so the parent form can gate Review. */
export function subjectsValid(subjects: readonly SubjectSummary[]): boolean {
  return subjects.length > 0 && subjects.every((s) => s.name.trim().length > 0)
}

function blankSubject(): SubjectSummary {
  return { kind: 'User', name: '', namespace: null }
}

const SELECT_CLASS =
  'h-8 min-w-0 rounded-none border border-input bg-transparent px-2.5 py-1 text-xs outline-none focus-visible:border-ring focus-visible:ring-1 focus-visible:ring-ring dark:bg-input/30'

export function SubjectEditor({ subjects, onChange }: SubjectEditorProps) {
  const update = (i: number, patch: Partial<SubjectSummary>) => {
    const next = subjects.map((s, idx) => (idx === i ? { ...s, ...patch } : s))
    onChange(next)
  }
  const remove = (i: number) => {
    onChange(subjects.filter((_, idx) => idx !== i))
  }
  const add = () => {
    onChange([...subjects, blankSubject()])
  }

  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-col gap-2">
        {subjects.map((subject, i) => {
          const nameEmpty = subject.name.trim().length === 0
          return (
            <div
              key={i}
              data-testid="rbac-subject-row"
              className="flex flex-col gap-1 border border-border/60 p-2"
            >
              <div className="flex flex-wrap items-center gap-2">
                <select
                  aria-label="Subject kind"
                  data-testid="rbac-subject-kind"
                  className={SELECT_CLASS}
                  value={subject.kind}
                  onChange={(e) => {
                    const kind = e.target.value
                    update(i, {
                      kind,
                      namespace: kind === 'ServiceAccount' ? (subject.namespace ?? '') : null,
                    })
                  }}
                >
                  {SUBJECT_KINDS.map((k) => (
                    <option key={k} value={k}>
                      {k}
                    </option>
                  ))}
                </select>
                <Input
                  aria-label="Subject name"
                  data-testid="rbac-subject-name"
                  placeholder="name"
                  className="w-40"
                  value={subject.name}
                  onChange={(e) => update(i, { name: e.target.value })}
                />
                {subject.kind === 'ServiceAccount' && (
                  <Input
                    aria-label="Subject namespace"
                    data-testid="rbac-subject-namespace"
                    placeholder="namespace"
                    className="w-40"
                    value={subject.namespace ?? ''}
                    onChange={(e) => update(i, { namespace: e.target.value })}
                  />
                )}
                <Button
                  type="button"
                  data-testid="rbac-subject-remove"
                  size="sm"
                  variant="ghost"
                  onClick={() => remove(i)}
                >
                  Remove
                </Button>
              </div>
              {nameEmpty && (
                <p data-testid="rbac-subject-error" className="text-xs text-destructive">
                  Subject name is required.
                </p>
              )}
            </div>
          )
        })}
      </div>
      <Button
        type="button"
        data-testid="rbac-subject-add"
        size="sm"
        variant="outline"
        className="w-fit"
        onClick={add}
      >
        Add subject
      </Button>
      {subjects.length === 0 && (
        <p data-testid="rbac-subject-error" className="text-xs text-destructive">
          At least one subject is required.
        </p>
      )}
    </div>
  )
}
