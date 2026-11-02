/** The clipboard, for Copy buttons. */

/** Copies `text`; `false` when there is no clipboard. */
export async function copyText(text: string): Promise<boolean> {
  try {
    await globalThis.navigator.clipboard.writeText(text)
    return true
  } catch {
    // No clipboard (an insecure context, or permission denied): the value is selectable anyway.
    return false
  }
}
