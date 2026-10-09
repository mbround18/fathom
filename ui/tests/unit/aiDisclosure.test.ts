/**
 * `needsHostedDisclosure`: the pure decision behind the one-time "this leaves your machine"
 * dialog in the Ask AI panel. Hosted providers prompt until accepted; Ollama never does.
 */
import { describe, expect, it } from 'vitest'

import { needsHostedDisclosure } from '@/features/ai-assist/disclosure'

describe('needsHostedDisclosure', () => {
  it.each(['github', 'claude', 'openai'] as const)('prompts for hosted provider %s', (active) => {
    expect(needsHostedDisclosure({ active, legacyMode: undefined, accepted: undefined })).toBe(
      true,
    )
    expect(needsHostedDisclosure({ active, legacyMode: undefined, accepted: false })).toBe(true)
  })

  it('never prompts for local Ollama, even with a legacy Anthropic mode lingering', () => {
    expect(
      needsHostedDisclosure({
        active: 'ollama',
        legacyMode: { type: 'Anthropic', data: { key_configured: true } },
        accepted: undefined,
      }),
    ).toBe(false)
    expect(
      needsHostedDisclosure({
        active: null,
        legacyMode: { type: 'Ollama', data: { endpoint: 'http://127.0.0.1:11434', model: 'x' } },
        accepted: undefined,
      }),
    ).toBe(false)
  })

  it('falls back to the 013 Anthropic mode when 038 reports no active provider', () => {
    expect(
      needsHostedDisclosure({
        active: null,
        legacyMode: { type: 'Anthropic', data: { key_configured: true } },
        accepted: undefined,
      }),
    ).toBe(true)
    expect(needsHostedDisclosure({ active: null, legacyMode: undefined, accepted: undefined })).toBe(
      false,
    )
  })

  it('stops prompting once accepted', () => {
    expect(needsHostedDisclosure({ active: 'claude', legacyMode: undefined, accepted: true })).toBe(
      false,
    )
  })
})
