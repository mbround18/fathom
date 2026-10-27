import type { WebSocketLike } from '@/protocol/transport'

/** A WebSocket stand-in the test drives by hand: `open()`, `serverClose(code)`, `sent`. */
export class FakeSocket implements WebSocketLike {
  static instances: FakeSocket[] = []

  readyState = 0
  onopen: ((ev: unknown) => void) | null = null
  onmessage: ((ev: { data: unknown }) => void) | null = null
  onerror: ((ev: unknown) => void) | null = null
  onclose: ((ev: { code: number }) => void) | null = null
  readonly sent: string[] = []
  readonly url: string

  constructor(url: string) {
    this.url = url
    FakeSocket.instances.push(this)
  }

  static reset(): void {
    FakeSocket.instances = []
  }

  static last(): FakeSocket {
    const s = FakeSocket.instances.at(-1)
    if (!s) throw new Error('no socket was opened')
    return s
  }

  send(data: string): void {
    if (this.readyState !== 1) throw new Error('send on a socket that is not open')
    this.sent.push(data)
  }

  close(code = 1000): void {
    if (this.readyState === 3) return
    this.readyState = 3
    this.onclose?.({ code })
  }

  open(): void {
    this.readyState = 1
    this.onopen?.({})
  }

  serverClose(code: number): void {
    this.close(code)
  }

  serverSend(frame: unknown): void {
    this.onmessage?.({ data: JSON.stringify(frame) })
  }

  frames(): unknown[] {
    return this.sent.map((s) => JSON.parse(s) as unknown)
  }
}
