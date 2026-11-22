/** A row drop owns one exact conversation, from the live event through
 * navigation and composer mount. No unscoped attachment queue is used. */
export interface RailDropSnapshot { key: string | null; version: number; page: string }
export interface RailFileDropDriver {
  snapshot(): RailDropSnapshot;
  open(key: string, current: () => boolean): Promise<boolean>;
  visible(): boolean;
  pending(key: string): boolean;
  attach(file: File, known: boolean): boolean;
  wait(): Promise<void>;
}

export type RailFileDropResult = 'attached' | 'changed' | 'unavailable' | 'rejected';
export async function attachFilesToRailChat(key: string, files: readonly { file: File; known: boolean }[], origin: RailDropSnapshot, driver: RailFileDropDriver): Promise<RailFileDropResult> {
  const unchanged = () => {
    const now = driver.snapshot();
    return now.version === origin.version && now.key === origin.key && now.page === origin.page;
  };
  if (!unchanged()) return 'changed';
  if (!files.length) return 'rejected';
  if (origin.key !== key || !driver.visible()) {
    // The workspace opener changes page to today before reading the chat.
    // It owns its navigation token; this guard owns only the old chat until
    // that opener commits the exact destination.
    const sameConversation = () => {
      const now = driver.snapshot();
      return now.version === origin.version && now.key === origin.key;
    };
    if (!await driver.open(key, sameConversation)) return 'unavailable';
  }
  let destination: RailDropSnapshot | undefined;
  // Bounded mount wait, not a global queue a different chat could consume.
  for (let attempt = 0; attempt < 100; attempt++) {
    const now = driver.snapshot();
    if (now.key === key) {
      if (!destination) destination = now;
      if (now.version !== destination.version || now.page !== destination.page) return 'changed';
      if (driver.visible()) {
        let rejected = false;
        for (const item of files) {
          const live = driver.snapshot();
          if (live.key !== key || live.version !== destination.version || live.page !== destination.page) return 'changed';
          if (!driver.attach(item.file, item.known)) rejected = true;
        }
        return rejected ? 'rejected' : 'attached';
      }
    } else if (destination || (!unchanged() && !(now.version === origin.version && now.key === origin.key && driver.pending(key)))) return 'changed';
    await driver.wait();
  }
  return 'unavailable';
}
