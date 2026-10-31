# Office server data for releases, feedback and the admin dashboard

> Dexterity Sheets Server keeps these files in its data folder (`C:\ProgramData\Dexterity
> Sheets\Server`). The server writes what the apps report; the Server Manager reads them for its
> dashboard and writes `updates/releases.json`. All files are UTF-8 JSON; a missing file is the
> same as an empty one, and a line that does not parse is skipped. Nothing here holds workbook
> contents.

These belong to the **application update system** and its feedback loop. Document versions
(`books/<id>/versions`) and sharing (`books/`) are separate and never touched by any of this:
marking a release blocked or rolling an app back changes no workbook.

## `updates/releases.json` — release status, channels, update policy

Written by the Server Manager (App updates › Releases), read by the server every 30 seconds.

```json
{
  "policy": { "mode": "prompt", "defaultChannel": "stable" },
  "releases": {
    "1.3.0": { "channel": "testing", "status": "ok", "note": "" },
    "1.2.2": { "channel": "stable", "status": "knownIssue", "note": "Sharing a workbook while typing could send it blank; fixed in 1.2.3." },
    "1.2.1": { "channel": "stable", "status": "blocked", "note": "Do not install: crashes on start on some PCs." }
  }
}
```

- `policy.mode`: `prompt` (the app offers the update; the person installs it, the default),
  `auto` (the app installs a newer version on its own when no workbook has unsaved changes), or
  `manual` (the app never offers; people use Automate › Check for Updates).
- `policy.defaultChannel`: the channel of an app that has not chosen one: `stable` or `testing`.
- `releases.<version>.channel`: `stable` (everyone) or `testing` (only apps on the testing
  channel). An installer with no entry is `stable`.
- `releases.<version>.status`:
  - `ok`;
  - `knownIssue`: still offered; apps running it show the note;
  - `deprecated`: no longer offered as an update; apps running it are told to move on;
  - `blocked`: never offered, and apps running it are told to roll back to the newest `ok`
    version older than it (the recommended rollback).
- `releases.<version>.note`: one or two sentences shown to people running that version.

## `clients.json` — installed versions

Written by the server when an app signs in and when it asks for updates.

```json
{ "clients": { "anita|PC-ENG-07": { "user": "anita", "machine": "PC-ENG-07", "version": "1.2.2", "channel": "stable", "lastSeen": 1793194586383 } } }
```

## `feedback/feedback.jsonl` — problem reports

One line per report, appended by the server when someone sends Help › Report a Problem (also
reached from the What's New dialog after an update, and from the rollback dialog).

```json
{"id":"fb-1793194586383-3","time":1793194586383,"user":"anita","machine":"PC-ENG-07","version":"1.2.2","os":"Windows 10 Pro 10.0.19045","features":["sharing","saving"],"description":"…","expected":"…","actual":"…","severity":"high","afterUpdate":true,"rolledBack":false,"logs":"…or null"}
```

- `features`: any of `opening`, `saving`, `formulas`, `formatting`, `importExport`, `sharing`,
  `notifications`, `startup`, `performance`, `other`.
- `severity`: `low`, `medium`, `high`, `blocking`.
- `logs`: the last part of the app's log (at most 64 KB) only when the person ticked "Include
  diagnostic logs"; the log records what the app did, never cell contents.
- Reports are grouped on the dashboard by `version` × feature.

## `health.jsonl` — update failures and app errors

One line per event the apps report:
`{"time":…, "user":"…", "machine":"…", "version":"1.2.2", "kind":"updateFailed"|"recovered"|"error", "detail":"…"}`.
`recovered` means the app started after a run that did not end normally and offered recovery of
unsaved work (crash or forced shutdown).

## `adoption.json` — feature adoption

Command use counted by the apps (command ids only, such as `home.bold` or `data.qualityCheck`;
never what they were used on), sent about once an hour, added up per version and day:

```json
{ "days": { "2026-10-28": { "1.2.2": { "home.bold": 412, "collab.share": 9 } } } }
```

Days older than 90 are dropped.
