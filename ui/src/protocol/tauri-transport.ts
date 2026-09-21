/** Tauri IPC transport: `send_cmd` / `subscribe_events` (ui/src-tauri/src/bridge.rs). */
import { Channel, invoke } from '@tauri-apps/api/core'

import type { Transport } from './transport'
import type { BackendCmd, EventBatch, SubscriptionInfo } from './types'

export class TauriTransport implements Transport {
  send(cmd: BackendCmd): Promise<number> {
    return invoke<number>('send_cmd', { cmd })
  }

  async subscribe(onBatch: (batch: EventBatch) => void): Promise<SubscriptionInfo> {
    const channel = new Channel<EventBatch>()
    channel.onmessage = onBatch
    return invoke<SubscriptionInfo>('subscribe_events', { onBatch: channel })
  }
}
