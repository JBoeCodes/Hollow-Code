# Active session motion

One indicator family for a conversation's live state (asks 10-11 A1/A2, replacing the
2026-10-07 doorway + light trails, which Alex found busy). Design note, options and
screenshots: the vault's `workspaces/local-f9ebfd0d1c780db6a000fcf2/outputs/hollow-asks-2026-10-11/lane-a.md`.

- `src/sessionIndicator.ts` (pure, tested): the states and their order, most urgent
  first: waiting on you > failed > running > queued (idle draws nothing); the label words
  ("Running · Claude"); a workspace roll-up (most urgent state, its count, every live chat
  named by state); `projectRollup`, a project row's roll-up that counts the lead, its
  subagents and other chats and says who ("Lead and 2 subagents running", 11-11);
  `sessionStateIndex`, which derives every conversation's state from the
  live evidence; and the Main agent's own states (resting, watching, working, waiting,
  paused by the cost cap).
- `src/store/sessionStates.ts`: one memo over signals that already update from events
  (chat events, the run queue, background chats, approvals, "waiting on you", Mission
  Control's feed). No polling. Failed = the conversation's newest queue row failed or was
  interrupted, or its latest feed turn ended with an error; opening it marks that failure
  seen.
- `src/components/SessionIndicator.tsx`: `SessionIndicator`, `SessionRollupBadge`,
  `MainAgentIndicator`. Every surface uses these, so a new look is this file and
  `src/styles/session-indicator.css`.
- Shapes, not colour, carry the state: open ring + one arc (running), dotted ring
  (queued), solid disc with a knocked-out "!" (waiting), ring with × (failed). No SVG
  ids. Forced colours draws everything in `CanvasText`.
- Motion: only the running arc rotates (1.6 s linear, a transform). It is not LIVE_WORK:
  it pauses with `--ambient-play` while the window is away, and both reduced-motion gates
  leave it still at one o'clock (still an open ring, distinct from queued).
- The provider moves into the label; idle rail rows still show the provider mark.

## Verification

Unit tests in `src/sessionIndicator.test.ts` and `src/mainAgentSpace.test.ts`; the rail
smoke spec (`tests/smoke/rail-git-status.spec.ts`) checks the role/label, the animation,
the pause while away, both motion gates and the collapsed roll-up. Not checked natively.
