import { describe, expect, it } from 'vitest'

import vectors from '../../../../../specs/060-keel-enroll-settings/contracts/template-vectors.json'
import { DEFAULT_TEMPLATE, MAX_BYTES, render, validate, type TemplateValues } from '@/features/settings/template'

const values: TemplateValues = vectors.values

describe('template (keel_core twin, contracts/template-vectors.json)', () => {
  it.each(vectors.render)('renders $template', ({ template, out }) => {
    expect(validate(template)).toBeNull()
    expect(render(template, values)).toBe(out)
  })

  it('substitutes in a single pass', () => {
    const v = vectors.single_pass
    expect(render(v.template, v.values)).toBe(v.out)
  })

  it.each(vectors.refuse)('refuses $template with $code', ({ template, code }) => {
    expect(validate(template)?.code).toBe(code)
  })

  it('refuses more than 4096 UTF-8 bytes before anything else', () => {
    expect(MAX_BYTES).toBe(4096)
    const long = '%t' + 'x'.repeat(vectors.too_long.bytes - 2)
    expect(validate(long)?.code).toBe(vectors.too_long.code)
    // Bytes, not characters: 1366 three-byte characters plus %t is 4100 bytes.
    expect(validate('%t' + '€'.repeat(1366))?.code).toBe('too_long')
    // Length is checked before placeholders.
    expect(validate('%x' + 'x'.repeat(4096))?.code).toBe('too_long')
    expect(validate('%t' + 'x'.repeat(4094))).toBeNull()
  })

  it('copies unknown placeholders and a trailing % through when rendering', () => {
    expect(render('%t %x %', values)).toBe(`${values.t} %x %`)
  })

  it('the default template is valid', () => {
    expect(validate(DEFAULT_TEMPLATE)).toBeNull()
  })
})
