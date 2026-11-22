import type { Locator } from '@playwright/test';
import { MOVED_STATUS } from '../../src/canvas/askSend';

/**
 * Click Hollow's Send under an ask-canvas (`.chat-ask-send-btn`). A pointer
 * click in the half second after the frame changed height is ignored, and
 * Hollow says so under the frame (2026-10-15): the button moved, and may have
 * slid under a pointer aimed at something else. A test's click can land in
 * that half second (the kit loading, a field's message clearing), so this
 * clicks again after it, as the person is told to. A click that went is never
 * repeated: the status then says something else at once.
 */
export async function clickAskSend(button: Locator) {
  const status = button.locator('xpath=..').locator('.chat-ask-send-status');
  for (let i = 0; i < 4; i++) {
    await button.click();
    if (!(await status.allTextContents()).includes(MOVED_STATUS.text)) return;
    await button.page().waitForTimeout(600);
  }
  throw new Error('Send was refused as "changed size" 4 times in a row');
}
