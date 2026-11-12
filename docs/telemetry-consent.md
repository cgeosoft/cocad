# Telemetry and consent plan

Standard: App Telemetry and Consent, in the owner's notebook at `CGEO/SOFT/App Telemetry and Consent.md`
Standard reviewed: 2026-11-06

Status: approved by the owner on 2026-11-06. Merged to `main` and released in `v0.3.2` on 2026-11-07, on the instruction of the owner. Notice version 2, with the update check, released in `v0.4.0` on 2026-11-08. Open items are in the [Release gate](#release-gate). The standard is a draft, and a lawyer has not reviewed it.

The standard's template gives the full path of the standard. This file uses a short path, because repo rule 8 forbids personal paths and the repo has a public mirror.

## Current state

Audit date: 2026-11-06. Audit base: commit `ed48163` (tag `v0.3.1`) and the working tree. Commit `e987510` added the telemetry.

### Urgent: the published text denies what the released build can do

| Fact | Evidence |
|---|---|
| The `POSTHOG_API_KEY` secret was added at 20:46 UTC. | `gh secret list` |
| The `v0.3.1` build ran after that, with the key in its environment. | Log of run 42291503319 shows `POSTHOG_API_KEY: ***` |
| Release `v0.3.1` is public with 4 packages. Each had 0 downloads at audit time. | `gh release view v0.3.1` |
| The live privacy page still says "The desktop app sends no data." and "It has no telemetry". It shows "Last updated: November 5, 2026". | Fetched from cocad.cgeosoft.com |
| The live FAQ says "no telemetry". The live `llms.txt` also differs from the repo. | Fetched and compared with the repo |
| The repo has the corrected text since commit `e987510`, but this text is not deployed. | `diff` of repo and live pages |

The standard says: "No build may send data that the published text denies." Consent in `v0.3.1` still gates all sending. But the consent screen linked to a page that said that the app sends no data.

The binary of the release was not inspected, so the key in that binary is not confirmed directly. A non-empty key in the build environment is the evidence.

Actions on 2026-11-06, approved by the owner:

- The repo website was deployed. The live privacy page, FAQ and `llms.txt` no longer say that the app has no telemetry.
- Release `v0.3.1` is now a draft, with 0 downloads. `v0.3.0` is the latest public release. The `POSTHOG_API_KEY` secret stays in place.

The table above and the rest of this audit describe the state before these actions.

### What the app does now

- `src/telemetry.rs` sends JSON directly to PostHog with `ureq`. It uses no PostHog SDK, so the `posthog-node` notes in the standard do not apply.
- The app has two boolean options in `<config>/config`: `analytics`, `error_reports`. A third boolean, `privacy_asked`, records that the user answered.
- A build with no valid key sends nothing, does not ask, and hides the Settings card.
- Only the GUI path installs the panic hook and calls `telemetry::apply`.
- Events are `app_started`, `model_opened`, `model_exported`, `view_changed`, `photo_saved` and `$exception`.
- Error kinds are `panic`, `renderer_failed`, `export_failed` and `photo_save_failed`.

### Website

`extras/website/public/analytics.js` loads `posthog-js` only after the visitor accepts. The site has analytics only, so it needs one option. It has no error reports.

## Outbound connections

### Desktop app

| Connection | When | Data | Disclosed now | Note |
|---|---|---|---|---|
| `https://eu.i.posthog.com/i/v0/e/` | After consent, in the window process only | Events and error reports | Yes, but with wrong detail | The only network call of the audit date. `src/update.rs` added the GitHub call below |
| `https://api.github.com/repos/cgeosoft/cocad/releases` | Window process only, at window start and each hour (each 24 hours before 2026-11-12), unless the user turns off the update check | IP address, app version in `User-Agent` | Yes | Added 2026-11-07. Not part of analytics or error reports, so it has no consent option. GitHub is a new recipient of the IP address, so the owner raised `NOTICE_VERSION` to 2 on 2026-11-08. The app and the site ask again. The Privacy Notice lists the connection |
| DNS lookup for that host | Before each new connection | Host name, through the system resolver | No | Add to the Notice as part of the connection |
| System browser: GitHub repo, privacy page | Only when the user clicks a link | Normal browser request | No | The app opens the link. It does not fetch it |
| `127.0.0.1` on a free port (MCP) | When the MCP option is on | Tool calls, with a random token | Yes | Loopback only. It stays on the computer |
| MCP client to its model provider | When the user connects a client | What the client reads | Yes | Not a CoCAD connection |
| `openscad` child process | Each render | Model file, on the local computer | Partly | See below |

These checks found no other network path:

- `Cargo.lock` has `ureq` and no other HTTP client.
- The code has no ads, remote fonts or data providers.
- `ui::fonts` reads a local font file, or uses the embedded fonts.
- `src/examples.rs` (uncommitted) writes embedded models. It makes no connection.

**The OpenSCAD process.** CoCAD runs a binary that the user picks. `openscad` links `libQt5Network`. `openscad-nightly` links `libQt6Network` and `libcurl-gnutls`. A preload library logged `connect` and `getaddrinfo` during an STL export of `examples/demo-box.scad` with both binaries on Linux. Neither binary made a call. A control run proved that the library logs. This covers the CLI export on Linux only, and the user can pick another binary. The Notice says so.

### Website

| Connection | When | Data | Disclosed now |
|---|---|---|---|
| Cloudflare Pages | Each visit | IP address, browser data, requested pages | Yes |
| `api.github.com` | Each visit, at most one time in 10 minutes per browser | IP address, browser data | Yes |
| `eu-assets.i.posthog.com` | After consent | SDK script request, IP address | Partly. The page names PostHog but not this load |
| `eu.i.posthog.com` | After consent | Events, IP address | Yes |
| Cloudflare scripts in `/cdn-cgi/` | Each visit on the live site | Same origin. Cloudflare adds them: email protection and a challenge script | Now yes. They are not in the repo |
| Links to GitHub, openscad.org, cgeosoft.com | Only when the visitor clicks | Normal browser request | Not needed |

## False or incomplete public claims

Severity: **false** means that the text is not true now. **Incomplete** means that a required item is missing. **Unverified** means that no evidence supports the claim yet.

### Live site, not the repo

| Place | Claim | Finding |
|---|---|---|
| Live privacy page, summary and app section | "The desktop app sends no data." "It has no telemetry" | **False** for a keyed `v0.3.1` build |
| Live privacy page, meta description | "The desktop app sends no data." | **False** |
| Live FAQ | "CoCAD has no account, no server and no telemetry." | **False** |
| Live `llms.txt` | "The app has no telemetry." | **False** |

### Repo text

| Place | Claim | Finding |
|---|---|---|
| Privacy page, `README.md:228`, consent text, `src/telemetry.rs` header | The app does not send "model contents, file names, parameter values or OpenSCAD output" | **False for error reports.** `scrub` removes only words with a slash. See the canary results below. |
| Privacy page, `README.md:226`, `ERRORS_NOTE` | "error text with file paths removed" | **False as a guarantee.** A name with no slash stays. Rule 21 also forbids the text |
| Privacy page | "The two random IDs are different, so an error report does not link to your analytics." | True for analytics. But all reports of one run share one ID, so reports link to each other |
| Privacy page, `README.md:228` | "The command line and `cocad mcp` send nothing." | **Incomplete.** `cocad mcp` can start a window (`launch` in `src/mcp.rs`). MCP calls run in the window and fire `model_opened` and `view_changed` (`src/app/remote.rs:187`, `:588`) |
| Privacy page | "so the stored events do not hold your address" | **Unverified.** It depends on PostHog behaviour for a client `$ip` value. The PostHog pages that I read do not describe this. Only "Discard client IP data" has documented behaviour |
| `README.md:230` | "`scripts/build.sh` also reads it from `extras/website/.env`" | **False in effect.** The `.env` value has quotes. `build.sh` keeps the quotes, so `key()` rejects it and a local release build has no key |
| Privacy page | "PostHog keeps the events for the retention period of the project." | **Incomplete.** Rule 34 needs a stated period that a vendor setting enforces |
| Privacy page | "To ask for the removal of data that PostHog already has, write to the contact address." | **Incomplete.** The page does not ask for the install ID, and the app does not show it. See rule 18 |
| Privacy page | "the app removes their random ID from your computer" | True, but it makes erasure impossible. See rule 18 |
| Privacy page | "These companies can process data outside the EU" | **Incomplete.** It does not give the hosting location or the transfers per recipient |
| Website README and privacy page | "This website sets no cookies." | **Unverified for the live site.** A `HEAD` request returned no `Set-Cookie`, but Cloudflare adds a challenge script. Check in a browser |
| `extras/website/README.md` | "It sets no cookies" | Same as above |
| `llms.txt` | "Privacy policy: Website analytics and storage" | **Incomplete.** The page also covers the app |
| `sitemap.xml` | `lastmod` 2026-11-05 for the privacy page | Out of date |
| All | The name "Privacy policy" | Use "Privacy Notice", to match rule 31 |
| Consent text, Settings | The notes describe error text and a random ID | Must change with the new error report |

### Privacy Notice checklist (repo page)

| Item | State |
|---|---|
| Controller identity and contact | Present |
| What each option sends | Present, but wrong for error reports |
| One example of each | **Missing** |
| Purpose and lawful basis | Present |
| Recipient, hosting location, transfers | **Incomplete**: "EU cloud" only |
| Retention period | **Missing** |
| Access, erasure, withdrawal | Present |
| Complaint to a supervisory authority | Present |
| How to make a request, install ID needed | **Missing** |
| Error reports have no ID, so we cannot find them | **Missing**. It says the opposite in effect |
| Separate from Terms | Present. CoCAD has no Terms. `LICENSE` is the MIT text, so no Terms change is needed |

### Canary results for `scrub`

I ran the real function from `src/telemetry.rs` in a scratch copy.

| Input | Output |
|---|---|
| `cannot read secret plan.stl: Permission denied` | Same text. The file name stays |
| `ERROR: Parser error in file "client-acme-bracket.scad", line 3` | Same text. The file name stays |
| `invalid value Project Falcon for width` | Same text. The value stays |
| `token=abc123 failed for ann@example.com` | Same text. The token and the email address stay |
| `cannot write /home/ann/my box.stl` | `cannot write <path>` |

A panic from `Result::unwrap` has the error value in its text. I tested this: the payload was `called Result::unwrap() on an Err value: "secret plan.stl is locked"`. A panic can therefore carry user text to `scrub`.

## Gaps against each rule

"App" means the desktop app. "Site" means the website. The standard applies the Consent, Data and Transparency rules to the site.

### Consent

| Rule | App | Site | Change |
|---|---|---|---|
| 1 Two options | Met | Not applicable: no error reports | None |
| 2 Off by default | Met | Met | None |
| 3 Nothing before consent | Met for one window. **Gap**: see rule 6 | Met | One write path, see below |
| 4 Separate step | Met: own modal, before the tour | Met | Keep the order |
| 5 No loss, no nag | Met | Met. The box returns after 12 months | Keep |
| 6 Equal withdrawal, at once | **Gap.** `cfg` is read one time per window. Window A turns an option off. Window B sends until it restarts. If B then saves Settings, it writes its old value and turns the option on again (`src/app.rs:2278`) | Met | Move consent out of `Config`. Each window reloads it |
| 7 Record each choice | **Gap.** Booleans only: no time, source or version | **Gap.** Stores value and time, no source or version | New consent record |
| 8 Ask again on change | **Gap.** No version, so no new prompt | **Gap.** The key name has `_v1` only | `NOTICE_VERSION` constant |
| 9 Owner only | Met. No MCP tool reads or writes consent, and `remote.rs` uses `cfg.mcp` only. **Keep this with a test** | Not applicable | Add a test |

### Data

| Rule | App | Site | Change |
|---|---|---|---|
| 10 Allowlist | **Gap.** `event(name, props: Value)` takes any JSON | **Gap.** The SDK adds URL, referrer and browser fields | Typed events in the app, final key filter in `post`. `before_send` filter on the site |
| 11 No personal data | **Gap.** See the canary results | **Gap.** `$current_url` and `$referrer` are free text | Remove error text. Filter site fields |
| 12 Fixed names and values | **Gap.** Names are strings at call sites. `format` comes from the file extension. `export::from_path` limits it on success, but the type does not | `demo_prompt.model` needs a check against `MODELS` | Enums |
| 13 No "anonymous" | Met: no such text found | Met | Keep |
| 14 No autocapture | Met: no SDK | **Gap.** `capture_pageview: true` is page view autocapture | Set it to false. Send a named `page_view` event |
| 15 No development builds | **Gap.** `option_env!` reads the shell. A debug build with the variable set sends data | Met: `wrangler pages dev` has no key | Add `cfg!(debug_assertions)` to `key()` |

### Analytics (app only)

| Rule | State | Change |
|---|---|---|
| 16 Random ID | Met: 16 bytes from `getrandom`, no hardware data | None |
| 17 One project per app | **Unknown.** `build.sh` takes its key from the website `.env`, so app and site may share a project. `site: cocad` suggests a shared project. I cannot read the CI secret | Owner chose one project. See [Owner input](#owner-input) |
| 18 ID in Settings, Copy, Reset | **Gap.** No display. Turning analytics off deletes the ID, so the user loses the only key to their data | See [Consent storage](#consent-storage-and-write-path) |
| 19 `consent_version` on each event | **Gap** | Add |

### Error reports (app only)

| Rule | State | Change |
|---|---|---|
| 20 New ID per report, no profile | **Gap.** `run_id` is one per start. Profile: met, `$process_person_profile` is false | New ID per report |
| 21 No error message | **Gap.** `value` holds the scrubbed text | Remove. `value` holds the capture code |
| 22 App frames only | **Gap.** All frames stay, with function names from `egui` and `std`. No pattern check | `[external]` and a strict pattern |
| 23 No paths, source, logs | Met for paths: `short_path` removes folders. The app sends no source lines or logs | Keep the tests |
| 24 New object from safe fields | **Gap.** `exception()` takes the text as an input | New `Report` type with no text field |
| 25 No vendor auto capture | Not applicable: no SDK. The panic hook is own code | Record in the code header |
| 26 Final allowlist | **Gap.** No filter | Add a key filter in `post`, since no SDK hook exists |
| 27 Rate limit | **Gap.** No limit. A repeated panic on a thread sends each time | One per fingerprint, 20 per run |
| 28 No report of a reporter failure | **Gap.** The hook has no guard. A panic in `post` enters the hook again. A panic on the queue thread is also reported | Guard flag, skip the queue thread |

The report also lacks `consent_version`, the release channel and the process field.

### Local data

| Rule | State | Change |
|---|---|---|
| 29 Local logs | Met: the log panel stays local | None |
| 30 Manual send | Not present. It is optional | None |

### Transparency

| Rule | App | Site | Change |
|---|---|---|---|
| 31 Separate Notice | Met in the repo, **not deployed** | Same | Deploy |
| 32 Links | Met: consent screen and Settings. No Terms exist | Met on the main page. **Gap**: `404.html` has no link | Add the link |
| 33 All connections listed | **Gap.** See [Outbound connections](#outbound-connections) | **Gap.** Cloudflare scripts | Add |
| 34 Enforced retention | **Gap** | **Gap** | Up to 12 months. See [Release gate](#release-gate) |

Other website item: the banner gives Accept the `btn-primary` style and Decline the plain style. The standard asks for equal weight. Use the same style for both.

### Other defects found

- If `Config::save` fails, the new choice applies for that run only. An old "on" value returns at the next start. The new write path must show the failure and apply "off" even if the write fails.
- The panic hook posts on the failing thread with a 5 second timeout, so a panic can hang for 5 seconds. Lower it to 2 seconds in the hook.
- CI puts the key in the environment of every job. A manual `workflow_dispatch` build gets the key too. Limit the key to the build steps of a tag.

## Consent storage and write path

**Storage.** A new file `<config>/consent` replaces the three keys in `Config`. This also stops the old-value overwrite from rule 6, because `Config::save` no longer writes consent.

```text
notice_version = 1
analytics = false
analytics_at = 2026-11-06T21:00:00Z
analytics_source = setup
error_reports = false
error_reports_at = 2026-11-06T21:00:00Z
error_reports_source = setup
```

- `source` is `setup`, `settings` or `update-prompt`.
- `NOTICE_VERSION` is one constant in `src/telemetry.rs`. It starts at 1. Raise it only when the data, the purpose or the recipient changes.
- Write the file through a temporary file and a rename. A half-written file reads as "off".

**Write path.** One function, `telemetry::set_consent(analytics, errors, source)`:

1. Reads the current record, and stamps only the options that changed.
2. Writes the file.
3. Applies the change. It applies an "off" even if the write fails, and it returns the error for a toast.

The consent window and the Settings window both call this function. Nothing else writes the file. No MCP tool can reach it.

**Other windows.** `App::poll` checks the mtime of the file at its existing poll interval, and calls `apply` on a change. The interval is the longest delay for a second window.

**Migration.**

- With no `consent` file and old keys present, read the old keys one time.
- `analytics = true` stays. The earlier screen named PostHog and "a random ID", so the standard's default keeps it.
- `error_reports = true` becomes off. The old screen described a different report.
- The record gets `notice_version = 0`, so the one-time prompt shows.
- The next `Config::save` drops the old keys.

**Prompt.** `consent` opens when the version is lower than `NOTICE_VERSION`. The boxes show the migrated values. The text says what changed. The window keeps one "Save choice" button and no close button, so yes and no have the same weight.

**ID (rule 18).** Proposal:

- Turning analytics off stops sending, and the ID stays in `analytics_id`.
- Settings shows the ID with Copy and Reset, also when analytics are off.
- Reset asks the user to copy the ID first, then deletes it.
- A deleted ID is not used again. `getrandom` makes a new one.

This changes the published sentence "the app removes their random ID".

**Website.** `cocad_consent_v2` stores `{ value, at, source, version }`. An old `v1` value is ignored, so the box shows one time more. `save` and `apply` stay the only write path.

## Analytics events and allowed properties

Each event is one variant of an `enum` in `src/telemetry.rs`. A method on the enum gives the name and the properties. `post` removes any key that is not on the list.

| Event | Properties | Values |
|---|---|---|
| `app_started` | `view`, `first_run` | `model`, `slice`, `photo`; boolean |
| `model_opened` | `parameters` | Count |
| `model_exported` | `format` | `stl`, `3mf`, `step` |
| `view_changed` | `view` | `model`, `slice`, `photo` |
| `photo_saved` | `samples` | Count |

Common properties: `consent_version`, `$lib`, `$lib_version`, `$app_version`, `$os`, `arch`, `source`, `$geoip_disable`.

- `$geoip_disable` stays. PostHog states that discarding the IP address does not remove location data.
- `$ip = 0.0.0.0` stays until the owner has confirmed "Discard client IP data". Public text then relies on that setting and does not mention `$ip`.
- Analytics events omit `$process_person_profile`, so PostHog makes a profile with no properties. The deletion runbook needs a person. Error reports set it to false.

Website events: `page_view`, `download_clicked` (`os`, `type`), `demo_prompt` (`model`), each with `site` and `consent_version`. The `before_send` filter removes all other properties, apart from the fields that the SDK needs in order to work.

## Error capture points

The standard says to report only uncaught failures in the first version. CoCAD has one such point:

| Point | Code | Note |
|---|---|---|
| Panic in the window process, on any thread | `window:panic` | `install_panic_hook` |

Remove `export_failed`, `photo_save_failed` and `renderer_failed`. These are handled errors. Remove `telemetry::error`, so that error text has no path to the reporter. The CLI and `cocad mcp` stay out of scope, because they do not install the hook.

**Report fields**

| Standard field | CoCAD value |
|---|---|
| ID | `distinct_id`, new from `getrandom` for each report |
| Error type | `$exception_list[0].type` = `panic`. Rust has no class name, so this is a fixed value |
| Capture code | `capture_code` = `window:panic` |
| Stack frames | `function`, `filename`, `lineno`, `colno` |
| Build | `$app_version`, `channel` = `stable` |
| Platform | `$os`, `arch` |
| Process | `process` = `window` |
| `consent_version` | The notice version in the record |

Fields that PostHog needs and the standard's table does not list:

- `$exception_list[0].value`: the capture code, not the message.
- `$exception_list[0].mechanism`: fixed values.
- `$exception_level` = `fatal`.
- `$process_person_profile` = false, `$geoip_disable` = true.

These hold only fixed values. I did not check which of them PostHog requires, so check this against a test project.

**Frames.** Keep a frame only if `function` passes `^cocad(::[A-Za-z_][A-Za-z0-9_]*)+$` and `filename` starts with `src/`. Other frames become `[external]`, and neighbouring `[external]` frames become one. The panic location frame follows the same rule.

The `.deb` is stripped, so its reports have the location frame only. The release profile has no debug data, so the tar and zip packages can have names but no file and line. Reports will be small. If this is not enough, debug symbols are a later step.

**Limit.** `fingerprint` is a hash of the type, the code and the top frame. The fingerprint stays on the device. Allow one report per fingerprint per run, and 20 per run. A guard flag stops a panic in the reporter from reporting itself.

## Text changes

Order: the standard's Delivery order, step 7. Publish before or with the release.

**Done on 2026-11-06:** the repo website was deployed. This fixed the live "no telemetry" text. The claims about error text needed the code change.

**With the release:**

| File | Change |
|---|---|
| `extras/website/public/privacy/index.html` | Rename to Privacy Notice. Add examples, hosting, transfers, retention, the request process, the install ID and the error report ID note. Replace the error row. Fix the `$ip` and MCP sentences. List DNS, links and the OpenSCAD result |
| `extras/website/public/index.html` | FAQ: match the new text |
| `extras/website/public/llms.txt`, `sitemap.xml` | Say that the Notice covers the app. New date |
| `extras/website/public/analytics.js`, `style.css` | Equal buttons, consent record, no autocapture, `before_send` filter, link text |
| `extras/website/public/404.html` | Add a Notice link |
| `extras/website/README.md` | New events, storage key and consent record |
| `README.md` | Privacy table, CLI and MCP sentence, `build.sh` sentence |
| `src/app.rs` | `ANALYTICS_NOTE`, `ERRORS_NOTE`, consent window, Settings ID row. Base the notes on the standard's text. Keep the line that names models, file names and parameter values |
| `src/telemetry.rs` | Header comment |
| `scripts/build.sh` | Remove quotes from the key |
| `.github/workflows/build.yml` | Limit the key to tag builds |
| `CLAUDE.md` | Rule 14: add `NOTICE_VERSION` and name the standard |
| `src/app/tour.rs` | Line 118 stays true. If the Settings screenshots change, run `scripts/tour-shots.sh`, as repo rules 5 and 6 say |

Do not use "anonymous", "no personal data", "offline" or "nothing leaves" in any text. Do not write "data never leaves the EU" until the sub-processor list is read.

**Conflict note.** `README.md`, `src/app.rs` and `src/app/tour.rs` have uncommitted changes from other work. Commit or merge those before the implementation starts.

## Owner input

Answers from the owner, 2026-11-06.

| Item | Value | Source |
|---|---|---|
| Controller legal name | Christos Georgiou | Default |
| Privacy contact | christos@cgeosoft.com | Default |
| Geographic address | Not needed. The app and the site earn no money | Owner |
| PostHog region | PostHog Cloud EU | Owner |
| PostHog project | One project for the app and the site | Owner |
| PostHog plan | Not stated. See [Release gate](#release-gate) | Open |
| Analytics retention | Up to 12 months | Default, owner |
| Error report retention | Up to 12 months. PostHog retention is per project, so the 90 day default cannot apply | Owner choice of one project |
| Earlier analytics consent | Kept, because the earlier screen disclosed the ID. It sends nothing until the prompt is answered | Recommendation accepted with the plan |
| Analytics person profile | Yes. Error reports have none | Recommendation accepted with the plan |
| Install ID when analytics are off | Stays. Reset removes it | Recommendation accepted with the plan |

**Rule 17.** The owner chose one project for the app and the site, as one product. The two use different random IDs and nothing links them. Other apps must not share this project. In one browser profile, a PostHog cookie from another CgeoSoft site had the domain `cgeosoft.com` and a different project key. So that site uses a different project.

### Release gate

Do not tag a release until the owner confirms each item. The published text depends on them.

- [ ] Sign the DPA.
- [x] The project is on PostHog Cloud EU (owner, 2026-11-06). The code uses `eu.i.posthog.com`.
- [x] (2026-11-07) Turn on "Discard client IP data" on the project. Console log capture is also off. The consent text and the Notice say that PostHog does not store the IP address.
- [ ] Confirm retention of 12 months. The Free plan has a fixed 1 year. A paid plan has a default of 2 years, so set it to 12 months.
- [x] Read the sub-processor list (2026-11-06). It names AWS and Modal (Germany for EU Cloud), Wiz (Germany, France), Cloudflare (global edge locations), Hiberly Ltd. (United Kingdom) and PostHog GmbH (Germany). The Notice says that data can leave the EU.
- [ ] Ask PostHog support how to delete events by install ID. Then add the deletion runbook.
- [ ] Check one error report in a test project. The docs do not say which `$exception` fields PostHog requires.
- [ ] Ask a lawyer to review the Notice. Include the transfer safeguards, which I did not verify, and `cocad_release_v1`, which the site stores before consent.
- [ ] Add the repository secret again if it changes. A tag build reads it. A manual build no longer does.

Publish the website with the release, not before: the new Notice describes the new error report.

## Implementation notes

These items differ from, or add to, the sections above.

- `Client` in `src/telemetry.rs` always exists, but it does nothing with no key or with both options off. This replaces "the client exists only if an option is on".
- The record is in `src/telemetry/consent.rs` and the report in `src/telemetry/report.rs`. `NOTICE_VERSION` is in `consent.rs`, and `telemetry` exports it.
- A choice counts only for the current `NOTICE_VERSION`. An old value shows in the prompt, but the app sends nothing until the user saves.
- A frame stays if its function passes the pattern and its file is safe or absent. A release build has names but no file. Without this rule, all frames of the tar and zip packages become `[external]`.
- The pattern removes `::{{closure}}` from the end of a name. `<cocad::app::App as eframe::App>::update` fails the pattern, so it becomes `[external]`.
- The report does not read the panic payload. The hook sends from a new thread and waits 2 seconds at most, because a panic inside a panic hook ends the process.
- On a failed write with an option turned off, the app also removes the consent file. The next start then asks again and sends nothing.
- On the website, `$session_id`, `$device_id`, `$set` and `$set_once` also go, so no field links a visit to a page address.
- The footer button is now `analytics`, because the site has no cookies.

## Tests

Stub the network. Tests must not reach the vendor. Put the send function behind a small trait, so that tests can capture the bodies.

| Test | Rule |
|---|---|
| Both options off: nothing is sent, also after a panic | 3 |
| Each option works alone | 1 |
| A toggle applies with no restart | 6 |
| A second window sees the change. A `Config::save` from that window does not change consent | 6 |
| A failed consent write still applies "off" | 6 |
| An old `error_reports = true` never turns on error reporting | Migration |
| An old `analytics = true` gives version 0 and a prompt | Migration, 8 |
| The write path records state, time, source and version, also for a no | 7 |
| No MCP tool changes consent | 9 |
| A canary puts a secret, a home path, a URL, a file name, an email address and model text into a panic message and a stack. The sent body has none of them | 11, 21, 23 |
| Keys of each event and report match the allowlist exactly | 10, 12 |
| The final filter removes an unknown key | 26 |
| Each report has a new ID that is not the install ID | 20 |
| `[external]` replaces other frames. A bad function name is dropped | 22 |
| The rate limit stops repeats, at one per fingerprint and 20 per run | 27 |
| A panic in the reporter makes no new report | 28 |
| A debug build sends nothing, also with a key in the environment | 15 |
| Each event has `consent_version` | 19 |
| Copy shows the ID. Reset makes a new one and warns first | 18 |
| Website: no PostHog request before consent. Decline removes `ph_*` keys. Sent properties match the list | 3, 6, 10 |
| Live site, in a browser: no cookies after a visit | Claim check |
| OpenSCAD CLI export with a logging preload library, for both binaries (manual) | 33 |

Use the `gui-test` skill for the window tests. Run `cargo test`, clippy and the Windows and macOS `cargo check` commands.

Tests are in `src/telemetry.rs`, `src/telemetry/consent.rs`, `src/telemetry/report.rs` and `extras/website/test/analytics.test.js`. Run the website tests with `bun test` in `extras/website`.

The website test uses a fake browser, so no test proves that `posthog-js` accepts the `before_send` result. Check one page view against a test project before the release.

### Window tests, 2026-11-07

These are manual runs on a private display on Linux, with private settings. The builds were release builds with a fake key and a local stub in place of PostHog. All of them passed.

| Test | Result |
|---|---|
| First start | Both boxes clear. 0 requests before the answer. After the answer, one `app_started` with `first_run: true` |
| Settings ID row | Shows the ID. Copy puts it on the clipboard. Reset warns first and removes it only after the second press. The next event makes a new ID |
| Turn analytics off | No request after that, and the ID stays. Only that option changes in the record. Turning it on again sends events with the same ID |
| Update prompt from 0.3.1 settings | Analytics selected, error reports clear, explanation shown. 0 requests, also with an MCP action while the prompt was open. The record then shows `update-prompt` |
| Old `config` keys | The next `Config::save` removes them. Consent does not change |
| MCP action after consent | The window sends `view_changed`. See [MCP events](#mcp-events-2026-11-12) for each tool |
| Second window | With analytics turned off in window B, an MCP action in window A sends nothing. A shows the new value in Settings. A save from A does not change the record |
| Panic, error reports on | One `$exception` with only the listed fields. The panic text is only in the local stderr. The process exits with code 101 |
| Panic, stripped binary as in the `.deb` | One report with `[external]` and the location frame only |
| Panic, error reports off | 0 requests |
| Panic with a stalled network | The process ended 2.5 seconds after the trigger: the poll delay of the test, then the 2 second wait |

The panic was added to a throwaway copy of the source. A real report still needs a check against PostHog. See [Release gate](#release-gate). I did not run these tests on Windows or macOS.

A second run, on the `telemetry-consent` branch after the merge of multicolor 3MF and File > Examples, passed all the tests above again. It added these results:

| Test | Result |
|---|---|
| File > Examples | One `model_opened` with the parameter count and no file name |
| Multicolor 3MF export through MCP | The tool writes the file. 0 requests |

### MCP events, 2026-11-12

This run checks the MCP text in `README.md` and the Privacy Notice. It used a release build of version 0.6.0 (commit `b2c9f3e`) with a fake key, a local stub in place of PostHog and a private display on Linux. The private settings had analytics on and error reports off. Each step counts the requests 3 seconds after the tool call. No tool call returned an error.

| MCP tool | Requests |
|---|---|
| `open_model` with no window | `app_started`, then `model_opened`. `cocad mcp` starts the window |
| `open_model` in an open window | One `model_opened` |
| `set_view` with a different `mode` | One `view_changed` |
| `set_view` with the same `mode` | 0 |
| `set_view` with `yaw`, `pitch`, `fit`, `wireframe` and `explode` | 0 |
| `save_photo`, in the model view and in the photo view | 0. The tool writes the PNG |
| `export_model` to STL, and to a multicolor 3MF with `slicer` | 0. The tool writes the file |
| `get_state`, `get_params`, `get_source`, `list_windows`, `set_params`, `reset_params`, `render`, `save_parameter_set`, `load_parameter_set`, `view_model`, `slice` | 0 |

I did not test `select_window`. It runs in the `cocad mcp` process, and in the window it calls only `get_state`.

The code agrees. In `src/app/remote.rs`, `open_model` calls `App::open` and `set_view` calls `App::set_mode`. These send `model_opened` and `view_changed`. The export and photo tools run on their own threads and have no telemetry call. The panic hook covers all threads of the window process except the `telemetry` threads. Thus a panic during an MCP action makes a normal error report when error reports are on. I did not test this with a panic.
