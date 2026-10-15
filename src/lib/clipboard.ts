// Copy spec: the draft to the clipboard. Never throws: a webview without the
// Clipboard API, or one that refuses the write (synchronously or not), reports
// false so the footer can say so instead of failing silently.

export interface ClipboardLike {
  writeText: (text: string) => Promise<void>;
}

const systemClipboard = (): ClipboardLike | undefined =>
  typeof navigator === "undefined" ? undefined : navigator.clipboard;

export async function copyText(text: string, clipboard: ClipboardLike | undefined = systemClipboard()): Promise<boolean> {
  if (!clipboard) return false;
  try {
    await clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}
