/**
 * Tiny HTTP CONNECT proxy for the hosted suite's WebKit project (spec 031, T038).
 *
 * Chromium maps *.localtest.me to the kind node with `--host-resolver-rules`; WebKit has no
 * such switch. It does honour a proxy, so this tunnels every CONNECT to `FATHOM_HOSTED_IP`
 * (same port) and refuses anything that is not *.localtest.me. TLS stays end to end.
 */
import net from 'node:net'

const ip = process.env.FATHOM_HOSTED_IP
const port = Number(process.env.RESOLVE_PROXY_PORT ?? 18443)
if (!ip) throw new Error('FATHOM_HOSTED_IP is required')

net
  .createServer((client) => {
    client.once('data', (head) => {
      const m = /^CONNECT ([^\s:]+):(\d+) HTTP\/1\.[01]\r\n/.exec(head.toString('latin1'))
      if (!m || !/(^|\.)localtest\.me$/.test(m[1])) {
        client.end('HTTP/1.1 403 Forbidden\r\n\r\n')
        return
      }
      const upstream = net.connect(Number(m[2]), ip, () => {
        client.write('HTTP/1.1 200 Connection Established\r\n\r\n')
        client.pipe(upstream).pipe(client)
      })
      const close = () => {
        client.destroy()
        upstream.destroy()
      }
      upstream.on('error', close)
      client.on('error', close)
    })
  })
  .listen(port, '127.0.0.1')
