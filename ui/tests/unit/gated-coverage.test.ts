/**
 * SC-003 (033): every mutating control must be wrapped in `<Gated>`. This parses every UI
 * component and fails on a mutating control (identified by its test hook) that has no `<Gated>`
 * ancestor. Adding a new mutating control means wrapping it, or (with a stated reason) listing
 * it in `NOT_CLUSTER_MUTATING`.
 */
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'

import ts from 'typescript'
import { describe, expect, it } from 'vitest'

const SRC = join(__dirname, '../../src')

/** Test hooks that name a control which mutates the cluster (or gates a mutation). */
const MUTATING =
  /^(action-|bulk-|node-action-(?!cancel|error)|port-forward-(action|restart)|helm-(release-uninstall|upgrade-apply)|rbac-binding-(create|edit)-open|role-assignment-open|yaml-apply(?!-status|-error)|teams-publish-profile)/

/** Mutating-sounding hooks that touch no cluster object (with the reason). */
const NOT_CLUSTER_MUTATING: Record<string, string> = {
  'port-forward-stop': 'tears down a local tunnel; writes nothing to the cluster',
  'node-action-cancel-drain': 'cancels an in-flight drain; only stops further evictions',
}

/** Controls that MUST be found (guards against the scan silently matching nothing). */
const REQUIRED = [
  'action-${a.id}',
  'bulk-${a.id}',
  'node-action-cordon',
  'node-action-uncordon',
  'node-action-drain',
  'node-action-taint-add',
  'node-action-taint-remove',
  'port-forward-restart',
  'port-forward-action-manual',
  'helm-release-uninstall',
  'helm-upgrade-apply',
  'rbac-binding-create-open',
  'role-assignment-open',
  'rbac-binding-edit-open',
  'yaml-apply',
  'teams-publish-profile',
]

function files(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const p = join(dir, name)
    if (statSync(p).isDirectory()) return files(p)
    return p.endsWith('.tsx') ? [p] : []
  })
}

function testIdOf(el: ts.JsxOpeningLikeElement): string | undefined {
  for (const attr of el.attributes.properties) {
    if (!ts.isJsxAttribute(attr) || attr.name.getText() !== 'data-testid' || !attr.initializer) {
      continue
    }
    const init = attr.initializer
    if (ts.isStringLiteral(init)) return init.text
    if (ts.isJsxExpression(init) && init.expression) {
      const e = init.expression
      if (ts.isNoSubstitutionTemplateLiteral(e)) return e.text
      if (ts.isTemplateExpression(e)) {
        return (
          e.head.text +
          e.templateSpans.map((s) => '${' + s.expression.getText() + '}' + s.literal.text).join('')
        )
      }
    }
  }
  return undefined
}

interface Found {
  file: string
  id: string
  gated: boolean
}

function isGatedAncestor(p: ts.Node): boolean {
  return ts.isJsxElement(p) && p.openingElement.tagName.getText() === 'Gated'
}

function scan(): Found[] {
  const out: Found[] = []
  for (const file of files(SRC)) {
    const text = readFileSync(file, 'utf8')
    const sf = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
    const visit = (node: ts.Node): void => {
      if (ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) {
        const id = testIdOf(node)
        if (id !== undefined && (MUTATING.test(id) || id in NOT_CLUSTER_MUTATING)) {
          const gated = ts.findAncestor(node.parent, isGatedAncestor) !== undefined
          out.push({ file: relative(SRC, file), id, gated })
        }
      }
      ts.forEachChild(node, visit)
    }
    visit(sf)
  }
  return out
}

describe('SC-003: mutating controls are capability-gated', () => {
  const found = scan()

  it('finds the known mutating controls (the scan is not vacuous)', () => {
    const ids = found.map((f) => f.id)
    for (const required of REQUIRED) {
      expect(
        ids.some((id) => id.startsWith(required)),
        `missing control ${required}`,
      ).toBe(true)
    }
  })

  it('fails on any mutating control not wrapped in <Gated>', () => {
    const ungated = found
      .filter((f) => !f.gated && !(f.id in NOT_CLUSTER_MUTATING))
      .map((f) => `${f.file}: ${f.id}`)
    expect(ungated).toEqual([])
  })
})
