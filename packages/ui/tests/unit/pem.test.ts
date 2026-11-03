import { describe, expect, it } from 'vitest'

import { checkPem, describePem } from '../../src/kit'
import { CERT, CHAIN, KEY } from './fixtures/pem'

describe('checkPem', () => {
  it('lists the blocks it finds', () => {
    expect(checkPem(CERT)).toEqual({ ok: true, blocks: [{ type: 'CERTIFICATE' }] })
    const chain = checkPem(CHAIN)
    expect(chain.ok && describePem(chain.blocks)).toBe('2 blocks: CERTIFICATE, CERTIFICATE')
    expect(checkPem(CERT.replace(/\n/g, '\r\n')).ok).toBe(true)
  })

  it('refuses a block of a type the field does not take', () => {
    expect(checkPem(KEY, ['CERTIFICATE'])).toEqual({ ok: false, error: 'Expected CERTIFICATE, found PRIVATE KEY.' })
    expect(checkPem(KEY, ['PRIVATE KEY', 'EC PRIVATE KEY']).ok).toBe(true)
  })

  it.each([
    ['', 'Paste or load a PEM file.'],
    ['hello', 'This is not PEM: it has no BEGIN line.'],
    [CERT.replace(/-----END CERTIFICATE-----\n/, ''), 'A block has no END line.'],
    [CERT.replace('END CERTIFICATE', 'END PRIVATE KEY'), 'A CERTIFICATE block ends as PRIVATE KEY.'],
    [CERT.replace('TUlJ', 'TU!J'), 'The CERTIFICATE block is damaged: its body is not base64.'],
    [CERT + '-----BEGIN CERTIFICATE-----\nabc', 'A block is cut short.'],
  ])('says what is wrong: %#', (text, error) => {
    expect(checkPem(text)).toEqual({ ok: false, error })
  })

  it('takes the headers an encrypted legacy key carries', () => {
    const legacy = KEY.replace('BEGIN PRIVATE KEY-----\n', 'BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\nDEK-Info: AES-128-CBC,00FF\n\n').replace(
      'END PRIVATE KEY',
      'END RSA PRIVATE KEY',
    )
    expect(checkPem(legacy)).toEqual({ ok: true, blocks: [{ type: 'RSA PRIVATE KEY' }] })
  })
})
