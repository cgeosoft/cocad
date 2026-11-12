# Privacy

CoCAD works without an account. Your models stay on your computer.

CoCAD can send two kinds of data to its developer. Both are optional, and both are off until you accept them.

## Your two choices

At the first start, CoCAD asks about two kinds of data. Both boxes start clear. You can change your choice at any time in **File > Settings > Privacy**. A change applies at once.

| Choice | Why | What CoCAD sends |
|---|---|---|
| **Usage analytics** | To see which features people use | App start, view changes, model open with the number of parameters, export format, photo save with the sample count, version, system, processor type, notice version, a random install ID |
| **Error reports** | To find and repair failures | Error type, a fixed failure code, location in the CoCAD source code, version, release channel, system, processor type, notice version, a new random ID for each report |

CoCAD asks again if the data, the purpose or the recipient changes.

## What CoCAD does not send

- The contents of your models
- File names
- Parameter values
- The output of OpenSCAD
- Error messages. An error report also contains no install ID.

## Who gets the data

PostHog receives the data for the developer. PostHog sees your IP address and does not store it.

## The update check

The update check is separate from these two choices. It sends no analytics. When CoCAD asks GitHub for the list of releases, GitHub sees your IP address and the app version. To stop it, turn off **Check for new versions** in **File > Settings > Updates**. See [Settings](Settings#updates).

## When an AI assistant is connected

If you connect an [AI assistant](AI-assistant) and usage analytics are on, the window sends only these events for the actions of the assistant:

- App start, when the assistant starts the window.
- Model open, each time the assistant opens a model.
- View change, when the assistant changes between the Model, Slice and Photo tabs.

The other actions of the assistant, such as an export or a photo, send no event. If error reports are on, the window reports a failure during an assistant action in the same way as other failures.

## Delete your data

**File > Settings > Privacy** shows your install ID, with **Copy** and **Reset**. A request to delete your analytics data needs this ID.

The full legal text is the [Privacy Notice](https://cocad.cgeosoft.com/privacy/) on the website.

---

**For technical readers.**

- Only the app window sends data. The `params`, `export`, `slice` and `photo` commands and the `cocad mcp` process send nothing.
- With the MCP tools: `open_model` sends the app start and model open events, and `set_view` sends a view change when it changes the `mode`. The other tools send no event.
- A build reads the PostHog project key from `POSTHOG_API_KEY` at compile time. A debug build or a build with no key sends nothing and does not ask. See [Build from source](Build-from-source#analytics-key).
- The design of this code is in [`docs/telemetry-consent.md`](https://github.com/cgeosoft/cocad/blob/main/docs/telemetry-consent.md).
