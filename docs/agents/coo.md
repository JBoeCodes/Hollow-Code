# The COO page

Part of the agent manual; [AGENTS.md](../../AGENTS.md) is the entry point. Full reference: [COO](../COO.md).

- The COO (named Main agent until 2026-11-10) has one surface, its page; one conversation is on screen at a time. A new route to it calls `openMainAgent` and hands text to the page's composer (`setChatPrefillRequest` to fill the box, `setPendingChatSend` with `preset` to send it as typed). See [COO entry points](../COO.md#entry-points).
- It runs on any installed agent; its one extra tool rule and fresh-thread defaults: [COO provider](../COO.md#provider-and-model).
- Default standing orders are created only by an explicit GUI click through `create_main_standing_orders`: exclusive create, never seeded on page open, never overwriting an existing (even empty) file. Its command-map row and `USER_ONLY` coverage entry stay paired.
- The header's status read is independent of child chats, keeps last-good data with Retry, and refreshes on focus, saved settings, the next check-in and hourly while one is past due. Waiting items are the bell's, with their exact opener.
- Counts never show unknown as zero. Provider tint follows observed working chats and stays neutral for mixed or missing evidence.
- The Actions link requests a one-time focus/reveal at the lazy Automations disclosure after its navigation guard accepts; leaving cancels it, and remounts never replay it. Automations mounts `MainAgentActions` with retained evidence and Retry.
- COO cap accounting counts recorded spend during running successor chains; see [Usage budgets](../reference/runs.md#usage-budgets-paperclip-pr-8a).
- Presence, attention counts and night activity: [COO presence](../COO.md#coo-presence). Mission Control tracks: [Activity tracks](../COO.md#mission-control-activity-tracks).
