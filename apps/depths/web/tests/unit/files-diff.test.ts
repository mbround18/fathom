import { describe, expect, it } from 'vitest'

import { lineDiff } from '@/features/files/diff'

describe('lineDiff (spec 053)', () => {
  it('is empty for the same text', () => {
    expect(lineDiff('a.txt', 'x\ny\n', 'x\ny\n')).toBe('')
  })

  it('shows one changed line with its context and line numbers', () => {
    const before = ['1', '2', '3', '4', '5', '6', '7', '8', '9'].join('\n') + '\n'
    const after = before.replace('5\n', 'five\n')
    expect(lineDiff('a.txt', before, after)).toBe(
      ['--- a/a.txt', '+++ b/a.txt', '@@ -2,7 +2,7 @@', ' 2', ' 3', ' 4', '-5', '+five', ' 6', ' 7', ' 8'].join('\n') + '\n',
    )
  })

  it('splits far-apart changes into separate hunks', () => {
    const lines = Array.from({ length: 30 }, (_, i) => String(i + 1))
    const after = [...lines]
    after[1] = 'two'
    after[27] = 'twenty-eight'
    const d = lineDiff('a', lines.join('\n'), after.join('\n'))
    expect(d.match(/^@@/gm)).toHaveLength(2)
    expect(d).toContain('-2\n+two\n')
    expect(d).toContain('-28\n+twenty-eight\n')
  })

  it('handles added and removed lines', () => {
    expect(lineDiff('a', 'a\nb\n', 'a\nb\nc\n')).toContain('+c\n')
    expect(lineDiff('a', 'a\nb\nc\n', 'a\nc\n')).toContain('-b\n')
    expect(lineDiff('a', '', 'new\n')).toContain('+new\n')
  })

  it('notes a change of the final newline alone', () => {
    expect(lineDiff('a', 'x\n', 'x')).toContain('final newline')
  })
})
