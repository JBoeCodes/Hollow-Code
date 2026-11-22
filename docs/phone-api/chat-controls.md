# Phone chat controls (v1.2)

2026-11-06. Additive to PHONE_CHAT_API.md. Pairing, tailnet identity, body limits,
no-store responses and send idempotency use the existing v1 guards.

## Read choices

`GET /v1/chat-controls?agent=codex` returns the installed agents and the selected
agent's model suggestions. Omit agent to select Hollow's default. Unknown or
unavailable agents return 400. Models come from the Mac's existing catalog;
source and warning describe its freshness. Custom model IDs remain supported.

Response fields: `agent`, `agents: [{id,label,available}]`,
`models: [{id,label}]`, `source`, `warning` (nullable),
`defaultModel` (nullable), `defaultEffort` (nullable), `effortLevels`,
`permissionModes: [{id,label,available,reason}]`. Reason is nullable.
No account, credential, filesystem path or transcript is included.
Catalog discovery uses the existing bounded CLI catalog loader. Clients allow
60 seconds for this read and keep the composer usable if it fails.

## Choose settings for a send

Both `POST /v1/sessions` and `POST /v1/sessions/{key}/messages` accept an optional
`controls` object: `{agent?, model?, effort?, permissionMode?}`.
The iPhone labels this **Send settings** and stages it until Send. Once a turn
starts, its agent and model become this chat's inherited choices for later phone sends.
Its permission is inherited by later sends unless the Mac has a saved permission
choice for this chat, which takes precedence. Effort applies to this send only.
No app-wide default or saved Mac picker choice changes. Clearing pending choices
does not undo an accepted turn's settings; choose the desired settings again.
The sheet explains inheritance and confirms Bypass all, including that later
messages can remain unrestricted. The composer names the pending choices.
An unanswered send locks settings and keeps them with its retry. Question-card
answers continue using the existing chat settings.

- Missing or null fields inherit existing behavior. A changed agent uses its
  own saved model, not the previous agent's model. Empty model means the chosen
  provider's default. Model IDs are at most 200 UTF-8 bytes, without whitespace
  or controls, leading dashes or invisible directional formatting. They are
  arguments, never shell fragments.
- Agent must be installed. The COO's workspace (`main`) takes any installed
  agent like every other since COO rebuild L6 (2026-11-10); it no longer
  forces or requires Claude.
- Effort is low, medium, high, xhigh or max. Hollow maps it to the closest level
  the model supports, as on the Mac. It travels with the queued turn and does
  not change the saved agent preference.
- Permission mode is plan, acceptEdits, auto or bypassPermissions. The Mac
  validates agent support; unavailable modes fail with 400 before queueing.
  Pairing authorizes the same permission choices as the person's composer.
  The UI explains the choice and labels Bypass all as unrestricted.
- Native resumption requires the same agent, unchanged model and compatible
  permission class. Otherwise the turn starts fresh with Hollow's retained
  conversation context, keeping the same Hollow chat key.
- Invalid controls return 400 without queueing. A busy chat remains 409.
  Reusing clientMessageId after acceptance with valid controls returns the
  original result. Clients lock settings while an outcome remains unknown;
  after a definite refusal they can change choices and mint a new id.

Workspace and Runs on selection remain the existing new-chat fields.
An older Mac returns 404 for the choices route. The iPhone retains its existing
agent/workspace/send controls and explains that additional settings need a
Hollow update. Explicit controls are preflighted again before Send; a 404 stops
the send, clears pending choices and keeps the draft with a visible notice.
An earlier send with an unknown outcome keeps its choices and retry identity
even if this capability check fails, so recovery cannot create a second chat.
It never silently drops explicit controls into an older send.

## Integration

Agent and model carry forward. Permission inherits unless the Mac has a saved
choice for the chat. Only effort is one-send. The same rule is stated above
under Choose settings for a send.

The What's new heading for inherited send settings and the matching
`paletteCoverage.test.ts` row stay the same string. A title change updates both.

The Main provider pre-check follows the production sender's workspace lookup.
A missing workspace brief alone does not block a legacy phone send. Main model
picks require Claude. Controls do not change Hollow's global settings.

Controls and attachment ids share the send routes. Keep both validations,
the attachment blocks, and the native-session reset policy. Attachment
fixtures skip hidden metadata files before comparing canonical paths.
Canvas answers inherit chat settings through an empty controls selection.

