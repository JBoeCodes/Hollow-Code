# Phone API v1.2: calendar

2026-11-19 (build run 11-17, row B3, ENG-219). One read-only route for the iPhone app's Today tab: today's meetings, as
the Mac's Agenda reads them from the selected calendar source (the Mac's own calendars or Microsoft 365). The same file
lives in both repositories (`docs/phone-api/calendar.md`), with the same fixtures in
`docs/phone-api-fixtures/calendar/`. Change it only in both at once. v1 and v1.1 are unchanged.

The route needs the bearer token and passes the same Host and Tailscale checks as v1. **Nothing here writes a file or
a calendar**, and it never asks macOS for calendar permission (a source Hollow can't read comes back as `error`). A
Hollow without this route answers 404 `not_found`; the app then hides the meetings section.

## `GET /v1/calendar[?from=<time>&to=<time>]`

- `from` and `to`: RFC 3339 times (`2026-11-19T00:00:00-05:00`), both or neither. Neither means the Mac's local day,
  midnight to the next midnight. `to` is after `from`, at most 8 days later. 400 `bad_request` otherwise, also for a
  name sent with nothing readable after it. Send `Z`, or percent-encode a `+` offset as `%2B`; a literal `+` is read as
  the plus it was meant to be.

200 (fixtures `calendar/day.json`, `calendar/work-data-off.json`):

```json
{
  "enabled": true, "connected": true, "complete": true, "partial": null, "error": null,
  "source": "Microsoft 365 · read only", "fetchedAt": "2026-11-19T14:02:11+00:00",
  "from": "2026-11-19T05:00:00Z", "to": "2026-11-20T05:00:00Z",
  "events": [{"id": "AAMk...", "title": "Standup", "start": "2026-11-19T09:30:00-05:00", "end": "2026-11-19T09:45:00-05:00",
              "allDay": false, "cancelled": false, "location": "Room 2", "organizer": "Dee", "joinUrl": "https://teams.example/j/1"}]
}
```

- `enabled`: **false when the companion's "include work data" is off** (Settings on the Mac), or its scope file is
  missing or unreadable: meetings are work data, so the route fails closed. Nothing was read and `events` is empty; the
  app says to turn work data on in Hollow on the Mac, not that the day is free. With `enabled` false the other flags
  (`connected`, `complete`) mean nothing: no source was asked.
- `connected`: Hollow has a source it could read; false says to connect a calendar on the Mac. `complete`: the read
  covered the whole window; false, with the reason in `partial`, when it stopped early. `error`: the Agenda's own words
  when the read failed (permission denied, a Microsoft sign-in that expired). `source`, `fetchedAt`: where and when.
- `events`: at most 100, in the source's order. `start` and `end` are as the source wrote them: an instant with an
  offset, or a civil time (no offset) for an all-day event, read as the Mac's Agenda reads them. `title` at most 200
  characters, `location` 200, `organizer` (a name) 120; `joinUrl` only when it is an `https` link. Attendees, bodies and
  web links are never sent.
