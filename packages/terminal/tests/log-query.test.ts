// @vitest-environment node
import { describe, expect, it } from 'vitest'

import { fieldsOf, LineCache, normalizeLevel, parseLine, prettyJson, textLevel } from '../src/log-parse'
import { filterLines, highlightRanges, matchesQuery, parseQuery } from '../src/log-query'

const E = '\u001b'

describe('level detection', () => {
  it.each([
    ['{"level":"error","msg":"boom"}', 'error'],
    ['{"lvl":"WARN","msg":"x"}', 'warn'],
    ['{"severity":"INFO","message":"x"}', 'info'],
    ['{"level":50,"msg":"pino"}', 'error'],
    ['{"level":30,"msg":"pino"}', 'info'],
    ['{"log":{"level":"debug"},"message":"ecs"}', 'debug'],
    ['{"@l":"Warning","@m":"serilog"}', 'warn'],
    ['2026-10-23T12:00:00Z {"level":"fatal","msg":"after a timestamp"}', 'fatal'],
    ['app-1  | {"level":"trace"}', 'trace'],
    ['time=2026 level=warn msg="disk low"', 'warn'],
    ['ts=1 lvl="error" msg=x', 'error'],
    ['E1023 12:00:00.123456    1 controller.go:42] sync failed', 'error'],
    ['W1023 12:00:00.123456    1 reflector.go:1] watch closed', 'warn'],
    ['2026-10-23T12:00:00Z I1023 12:00:00.000001 1 main.go:1] started', 'info'],
    ['2026/10/23 12:00:00 [error] 7#7: *1 connect() failed', 'error'],
    ['[warn] something odd', 'warn'],
    ['<debug> poll', 'debug'],
    ['2026-10-23 12:00:00.000  INFO 1 --- [main] o.s.Boot : Started', 'info'],
    ['WARNING:root:deprecated call', 'warn'],
    ['ERROR: relation "x" does not exist', 'error'],
    ['error: could not open file', 'error'],
    ['panic: runtime error: index out of range', 'fatal'],
    [`${E}[31mERROR${E}[0m coloured level`, 'error'],
  ])('%s → %s', (line, want) => {
    expect(parseLine(line).level).toBe(want)
  })

  it.each([
    'GET /health 200',
    'the error count is 0 (prose, deep in a message is just a word but lower-case here)',
    '{"status":500,"msg":"http status is not a level"}',
    '{"severity":17,"msg":"otel number is another scale"}',
    'ERR_CONNECTION_REFUSED is a code, not a level',
  ])('no level: %s', (line) => {
    expect(parseLine(line).level).toBeNull()
  })

  it('normalizes spellings and numbers', () => {
    expect(normalizeLevel('Warning')).toBe('warn')
    expect(normalizeLevel('CRIT')).toBe('fatal')
    expect(normalizeLevel('notice')).toBe('info')
    expect(normalizeLevel(10)).toBe('trace')
    expect(normalizeLevel('60')).toBe('fatal')
    expect(normalizeLevel('nonsense')).toBeNull()
    expect(normalizeLevel({})).toBeNull()
  })

  it('looks for text levels only near the start', () => {
    expect(textLevel(`${'x'.repeat(300)} ERROR`)).toBeNull()
  })
})

describe('fields', () => {
  it('flattens JSON, lower-cased, with dotted paths', () => {
    const f = fieldsOf(parseLine('{"Service":"API","http":{"status":503,"path":"/v1/Users"},"ok":false,"n":null}'))
    expect(f.get('service')).toBe('api')
    expect(f.get('http.status')).toBe('503')
    expect(f.get('http.path')).toBe('/v1/users')
    expect(f.get('ok')).toBe('false')
    expect(f.get('n')).toBe('null')
  })

  it('reads logfmt, quoted values included', () => {
    const f = fieldsOf(parseLine('level=info msg="hello \\"world\\"" user=bob dur=12ms'))
    expect(f.get('msg')).toBe('hello "world"')
    expect(f.get('user')).toBe('bob')
    expect(f.get('dur')).toBe('12ms')
  })

  it('does not parse a broken or huge JSON line', () => {
    expect(parseLine('{"level":"error",').json).toBeNull()
    expect(parseLine(`{"a":"${'x'.repeat(70_000)}"}`).json).toBeNull()
    expect(parseLine('[1,2,3]').json).toBeNull()
  })

  it('escapes C1 controls and bidi overrides in pretty JSON', () => {
    const pretty = prettyJson({ a: 'x‮y\u0085z\u001b' })
    expect(pretty).toContain('\\u202e')
    expect(pretty).toContain('\\u0085')
    expect(pretty).toContain('\\u001b')
    expect(pretty).not.toMatch(/[\u0080-\u009f‪-‮]/)
  })
})

describe('parseQuery', () => {
  it('reads free text, phrases, exclusions, levels and fields', () => {
    const q = parseQuery('Timeout "connection reset" -healthz -"GET /ready" level:ERR -level:debug service:API path:/v1/*')
    expect(q.terms).toEqual([
      { text: 'timeout', negate: false },
      { text: 'connection reset', negate: false },
      { text: 'healthz', negate: true },
      { text: 'get /ready', negate: true },
    ])
    expect(q.levels).toEqual(['error'])
    expect(q.notLevels).toEqual(['debug'])
    expect(q.fields.map((f) => [f.key, f.value, f.negate, f.pattern !== null])).toEqual([
      ['service', 'api', false, false],
      ['path', '/v1/*', false, true],
    ])
    expect(q.highlights).toEqual(['timeout', 'connection reset', 'api', '/v1/'])
    expect(q.empty).toBe(false)
  })

  it('keeps a quoted field value whole', () => {
    expect(parseQuery('msg:"disk low" x').fields[0]?.value).toBe('disk low')
  })

  it('is empty for blank input, and tolerates an unterminated quote', () => {
    expect(parseQuery('   ').empty).toBe(true)
    expect(parseQuery('"half open').terms).toEqual([{ text: 'half open', negate: false }])
    expect(parseQuery('-').terms).toEqual([{ text: '-', negate: false }])
    expect(parseQuery('--flag').terms).toEqual([{ text: '--flag', negate: false }])
  })

  it('makes an unknown level impossible rather than ignored', () => {
    const q = parseQuery('level:bogus')
    expect(q.impossible).toBe(true)
    expect(matchesQuery(parseLine('level=error x'), q)).toBe(false)
  })
})

describe('matching', () => {
  const m = (q: string, line: string) => matchesQuery(parseLine(line), parseQuery(q))

  it('free text and phrases match case-insensitively on the plain text', () => {
    expect(m('ready', `${E}[32mREADY${E}[0m on :8080`)).toBe(true)
    expect(m('"ready on"', `${E}[32mready${E}[0m on :8080`)).toBe(true)
    // The escape sequence is not searchable text.
    expect(m('[32m', `${E}[32mready`)).toBe(false)
  })

  it('exclusions drop lines', () => {
    expect(m('-healthz', 'GET /healthz 200')).toBe(false)
    expect(m('GET -healthz', 'GET /users 200')).toBe(true)
  })

  it('level terms match the detected level, any of several', () => {
    expect(m('level:error', '{"level":"error"}')).toBe(true)
    expect(m('level:error level:warn', '[warn] x')).toBe(true)
    expect(m('level:error', 'no level here')).toBe(false)
    expect(m('-level:debug', 'no level here')).toBe(true)
    expect(m('-level:debug', 'level=debug x')).toBe(false)
  })

  it('field terms match JSON and logfmt values, with wildcards', () => {
    expect(m('service:api', '{"service":"api"}')).toBe(true)
    expect(m('service:web', '{"service":"api"}')).toBe(false)
    expect(m('http.status:503', '{"http":{"status":503}}')).toBe(true)
    expect(m('path:/v1/*', 'path=/v1/users method=GET')).toBe(true)
    expect(m('-user:bob', 'user=bob')).toBe(false)
  })

  it('a key:value on a line without that field falls back to the literal text', () => {
    expect(m('http://example', 'fetching http://example.com')).toBe(true)
    expect(m('port:8080', 'listening on port:8080')).toBe(true)
  })
})

describe('filterLines', () => {
  const lines = [
    '{"level":"info","msg":"start"}',
    '{"level":"error","msg":"boom","service":"api"}',
    '[warn] slow request',
    'plain line',
    'level=error msg="db down" service=db',
  ]

  it('returns matching indices and per-level counts over the query, before the facet', () => {
    const cache = new LineCache()
    const r = filterLines(lines, parseQuery(''), { cache, facet: new Set(['error']) })
    expect(r.indices).toEqual([1, 4])
    expect(r.counts).toMatchObject({ info: 1, error: 2, warn: 1, none: 1 })
  })

  it('composes with the caller filter', () => {
    const cache = new LineCache()
    const r = filterLines(lines, parseQuery('level:error'), { cache, extra: (i) => i !== 1 })
    expect(r.indices).toEqual([4])
  })

  it('filters 50,000 lines within ~50 ms once parsed', () => {
    const cache = new LineCache(100_000)
    const big = Array.from({ length: 50_000 }, (_, i) =>
      i % 5 === 0
        ? `{"level":"${i % 10 === 0 ? 'error' : 'info'}","msg":"request ${String(i)}","service":"api","dur":${String(i % 300)}}`
        : `2026-10-23T12:00:00Z ${E}[32mINFO${E}[0m GET /items/${String(i)} 200 ${String(i % 97)}ms`,
    )
    const t0 = performance.now()
    filterLines(big, parseQuery(''), { cache })
    const cold = performance.now() - t0
    const queries = ['items/4', '"GET /items/1" -healthz', 'level:error', 'service:api -level:info', 'dur:1*']
    const times: number[] = []
    for (const q of queries) {
      const t = performance.now()
      const r = filterLines(big, parseQuery(q), { cache })
      times.push(performance.now() - t)
      expect(r.indices.length).toBeGreaterThan(0)
    }
    const worst = Math.max(...times.slice(1))
    console.info(
      `filterLines 50k: cold parse ${cold.toFixed(1)} ms; warm ${times.map((t) => t.toFixed(1)).join(' / ')} ms`,
    )
    // ~50 ms is the target on a developer machine; leave headroom for a loaded CI box.
    expect(worst).toBeLessThan(150)
  })
})

describe('highlightRanges', () => {
  it('finds, sorts and merges every occurrence', () => {
    expect(highlightRanges('abcabc', ['bc'])).toEqual([
      [1, 3],
      [4, 6],
    ])
    expect(highlightRanges('abcdef', ['abc', 'cde'])).toEqual([[0, 5]])
    // Touching ranges merge into one mark.
    expect(highlightRanges('aaaa', ['aa'])).toEqual([[0, 4]])
    expect(highlightRanges('x', [''])).toEqual([])
  })

  it('caps the number of ranges', () => {
    expect(highlightRanges('a b '.repeat(10_000), ['a'], 50)).toHaveLength(50)
  })
})

describe('LineCache', () => {
  it('parses once per string and stays bounded', () => {
    const cache = new LineCache(100)
    const a = cache.get('one')
    expect(cache.get('one')).toBe(a)
    for (let i = 0; i < 1_000; i++) cache.get(`line ${String(i)}`)
    expect(cache.size).toBeLessThanOrEqual(100)
  })
})
