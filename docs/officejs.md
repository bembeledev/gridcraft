# Office add-ins (Office.js)

> **Last reviewed:** 2026-10-16 · **Last updated:** 2026-10-16 · **Change:** minor (testing with Microsoft's samples in the real app: `cargo xtask office-samples`, the control channel's `office.*` methods, `__craftOfficeInfo`; previously minor: what Microsoft's samples need: their `office.js` never runs, colour names, function commands, events and the common API's enums; previously minor: an add-in whose `<Requirements>` this Office.js doesn't meet is blocked, with the reason; previously major: first version: Office add-in manifests as plugins, the `office.sync` bridge, task panes, GridCraft's own Office.js) · **Target:** Microsoft Excel (Microsoft 365)

GridCraft runs **Office add-ins**, the web add-ins built with the Office JavaScript API
(`Office.js`), on top of its own add-ins ([plugins.md](plugins.md)). An Office add-in is
a web app on its vendor's server, described by an XML manifest. GridCraft reads the manifest into
a plugin: its ribbon buttons and right-click items are ordinary plugin items, listed in File ›
Options › Add-ins with the others, and its commands open its **task pane** (or call a function of
its function file) instead of starting a program.

The task pane shows the add-in's page in the system web view (WKWebView on macOS, WebView2 on
Windows). Before the page's own scripts run, GridCraft gives it **GridCraft's Office.js**
(`crates/office-js`), an implementation of the public API written from Microsoft's published
reference. Microsoft's `office.js` is never read or shipped, and never runs in the pane: add-in
pages reference it (every one of Microsoft's samples does), and GridCraft answers those
references with an empty script ([below](#microsofts-officejs)). Each `context.sync()` of the
page becomes one `office.sync` command against the workbook.

```
 add-in page (vendor's https)              GridCraft
 ┌────────────────────────────────┐      ┌──────────────────────────────────────────┐
 │ the add-in's own scripts       │      │ office_pane (ui-egui): the pane, pump    │
 │   Excel.run(ctx => …)          │ JSON │ office.sync (engine): paths → objects,   │
 │   ctx.sync()                   │ ───► │   GridCraft commands, one undo step,     │
 │ GridCraft's Office.js (wasm)   │ ◄─── │   the manifest's permissions             │
 └────────────────────────────────┘      └──────────────────────────────────────────┘
   wry web view over the task pane (apps/gridcraft/src/office_web.rs)
```

## Installing an Office add-in

- Put the add-in's `manifest.xml` in a folder of the plugins folder (`<settings>/plugins/<name>/`),
  or load it with **Load from folder** / `plugins.load {"path": ".../manifest.xml"}`.
- The manifest must be for Excel (`<Host Name="Workbook"/>`), and its pages `https://`.
- It becomes the plugin `office.<its Id>`. Its default task pane (`<DefaultSettings><SourceLocation>`)
  is the command `taskpane`; each add-in command (`<VersionOverrides>`: buttons on Office tabs or
  custom tabs, menus, the cell's right-click menu) is a command of its own.

| Manifest | GridCraft |
|---|---|
| `<OfficeTab id="TabHome">` … `TabInsert`, `TabFormulas`, `TabData`, `TabReview`, `TabView`, `TabPageLayoutExcel`, `TabDraw`, `TabDeveloper` | That built-in tab (Developer → Automate); other ids → an **Add-ins** tab |
| `<CustomTab>` | A tab of its own, named by its `<Label>` |
| `<Control xsi:type="Button">`, the items of `xsi:type="Menu"` | Buttons (a menu's items are its group's buttons) |
| `<Action xsi:type="ShowTaskpane">` | Opens the page in the task pane (`TaskpaneId`s that match share a pane) |
| `<Action xsi:type="ExecuteFunction">` | Calls the function in the function file's page, with no pane ([function commands](#function-commands-executefunction)) |
| `<OfficeMenu id="ContextMenuCell">` | Items in the cell's right-click menu |
| `<Permissions>` | Restricted: settings only; ReadDocument / ReadAllDocument: read; WriteDocument: write; ReadWriteDocument: both |

Icons from the manifest aren't shown yet (the buttons get GridCraft's script icon). The unified
JSON manifest isn't read yet.

**Requirements.** The manifest's `<Requirements>` are checked when the add-in loads, against the
requirement sets and methods GridCraft's Office.js reports (below; one table,
`gridcraft_plugin::office::API_SETS` and `API_METHODS`, which `isSetSupported` answers from too).
`<Sets>` lists `<Set Name="ExcelApi" MinVersion="1.3"/>`; a set without `MinVersion` takes the
`<Sets DefaultMinVersion="…">` (else 1.1). `<Methods>` lists `<Method Name="Document.getSelectedDataAsync"/>`.
An unmet one makes the add-in **blocked**, as a plugin whose requirements aren't met is
([plugins.md](plugins.md#requirements-manifest-13)): it stays listed in File › Options › Add-ins
with the reason ("Needs ExcelApi 1.9; GridCraft supports up to 1.4", "Needs WordApi 1.1, which
GridCraft doesn't support"), and its buttons and menu items are absent, instead of failing at its
first call. `plugins.list` reports it as `state: "blocked"` with `blockedBy` (kind `feature`).

## What the API covers

Requirement sets reported as supported (`isSetSupported`, and the manifest check above):

| Set | Version | How much of it |
|---|---|---|
| ExcelApi | 1.4 | **A subset**: the objects in the table below. It is reported so that add-ins written for ExcelApi 1.1–1.4 load; what's missing (tables, charts, named items, bindings, events on worksheets…) throws `NotImplemented` when used |
| AddinCommands | 1.1 | All: ShowTaskpane and ExecuteFunction commands |
| DocumentEvents | 1.1 | All: `addHandlerAsync`/`removeHandlerAsync` with `DocumentSelectionChanged` |
| Settings, Selection, TextCoercion, MatrixCoercion | 1.1 | All |

Methods (`<Requirements><Methods>`): `Document.getSelectedDataAsync`, `setSelectedDataAsync`,
`addHandlerAsync`, `removeHandlerAsync`, `Settings.get`, `set`, `remove`, `saveAsync`. Other sets
(DialogApi, SharedRuntime, CustomFunctionsRuntime, RibbonApi, IdentityAPI, ExcelApi 1.5 and later…)
answer false, so add-ins that check `isSetSupported` turn the feature off; add-ins that require
them in their manifest are blocked (above). The table is one, `gridcraft_plugin::office::API_SETS`,
and a test keeps the library's copy equal to it.

| Object | Properties and methods |
|---|---|
| `Office` | `onReady` (waits for the library and for the page's HTML to be parsed, `DOMContentLoaded`, as Office does: pages whose scripts run in `<head>` look up their buttons in the callback), `initialize` (called once the library is ready, also when assigned later), `actions.associate`, `context.host`, `context.diagnostics`, `context.requirements.isSetSupported`, `context.document.getSelectedDataAsync` / `setSelectedDataAsync` (text, matrix), `context.document.addHandlerAsync` / `removeHandlerAsync` (`DocumentSelectionChanged`), `context.document.settings` (`get`, `set`, `remove`, `saveAsync`); the enumerations `AsyncResultStatus`, `CoercionType`, `EventType`, `HostType`, `PlatformType`, `Index`, `GoToType`, `BindingType`, `StartupBehavior`, `VisibilityMode`, `InitializationReason`, `ValueFormat`, `FilterType`, `SelectionMode`, `DocumentMode`, `ActiveView`, `FileType`, `Table` |
| `Excel.run`, `new Excel.RequestContext()` | `workbook`, `sync()` (one undo step; a failure takes the whole batch back), `trackedObjects` |
| `Workbook` | `worksheets`, `getSelectedRange()`, `getActiveCell()`, `name` |
| `WorksheetCollection` | `getActiveWorksheet()`, `getItem(name)`, `add(name?)` (at the end, the active worksheet stays; without a name the next "SheetN"), `items` |
| `Worksheet` | `name` (read, rename), `position`, `getRange(address?)`, `getCell(r, c)`, `getUsedRange()`, `activate()`, `delete()`, `set({name})` |
| `Range` | `values`, `formulas`, `numberFormat`, `text`, `address`, `rowCount`, `columnCount`, `rowIndex`, `columnIndex`, `cellCount`; `getCell`, `getOffsetRange`, `getResizedRange`, `getRow`, `getColumn`, `getEntireRow`, `getEntireColumn`, `getUsedRange`, `clear(applyTo)`, `select()`, `set({...})` (nested: `format.font`, `format.fill`) |
| `Range.format.font` | `bold`, `italic`, `underline`, `strikethrough`, `color`, `size`, `name`, `set({...})` |
| `Range.format.fill` | `color`, `clear()`, `set({color})` |

**Colours** are `#RRGGBB` (also `#RGB`) or an HTML colour name, in any case: the 148 CSS named
colours (`"yellow"`, `"LightBlue"`, `"rebeccapurple"`). Reading a colour answers `#RRGGBB`.

**Not there, but harmless:** the common API's `*Async` methods GridCraft lacks
(`goToByIdAsync`, `getFileAsync`, `getFilePropertiesAsync`, `bindings.*`, `addHandlerAsync` for
other events) call back with a failed result whose error code is `NotSupported`;
`ui.displayDialogAsync` with `NotImplemented`. The Promise-based `Office.addin.*`
(`showAsTaskpane`, `hide`, `getStartupBehavior`, `setStartupBehavior`, `onVisibilityModeChanged`),
`Office.ribbon.*` and the keyboard-shortcut methods of `Office.actions` reject with
`NotImplemented`. A page that calls them gets an error it can handle, never a `TypeError`.

One difference from Excel: a `sync()` is all or nothing. If an action fails, the earlier ones of
the same batch are taken back too, and the error says which action failed (`debugInfo`).

## Function commands (ExecuteFunction)

A button or menu item with `<Action xsi:type="ExecuteFunction">` calls a function of the add-in's
**function file** (`<FunctionFile resid="…"/>`), a page with no pane:

1. GridCraft loads the function file in a hidden web view (once; later commands reuse it), with
   its Office.js like any pane.
2. When the library has started there, it tells GridCraft (`{"event": "ready"}`); then GridCraft
   calls `__craftOfficeRunFunction({"call", "name", "control"})`. The library waits for the page
   to finish loading (its scripts have run), then finds the function: the one the page gave
   `Office.actions.associate(name, fn)`, else a global function of that name (older function files).
3. The function gets an event: `event.source.id` is the control's id in the manifest
   (`<Control id="Contoso.Button">`), and `event.completed()` tells GridCraft it has finished.
4. GridCraft waits up to **5 minutes** for `completed()`. A function that isn't there, that throws,
   or that never calls `completed()` is reported in a short message; nothing else is held up
   meanwhile.

The function's `Excel.run` and common-API calls go through `office.sync` with the add-in's
permissions, as a pane's do.

## Microsoft's office.js

Add-in pages load Microsoft's `office.js` with a `<script>` (from
`appsforoffice.microsoft.com`, `officeapis.public.onecdn.static.microsoft` or a copy of the npm
package `@microsoft/office-js`). It must not run in GridCraft: outside Office it never reports the
host, and it would replace the `Office` object the page's `Office.onReady` uses. So the bootstrap
(below), which runs before the page:

- gives every `<script>` whose source is one of those an empty script instead, whether it is in
  the page's HTML (the bootstrap watches the document as the parser builds it, and changes the
  source before the script would run) or made by the page's code (`script.src = …`,
  `setAttribute("src", …)`). The script's `load` event still fires, so loaders that wait for it
  carry on;
- defines `Office`, `Excel`, `OfficeExtension` and `OfficeRuntime` as properties that can't be
  replaced or deleted. Until the library has started they hold the page's early calls
  (`Office.onReady`, `Excel.run`, `Office.initialize`, `Office.actions.associate`) and hand them
  over; afterwards they are the library's objects. A script that assigns its own `Office` anyway
  is ignored, except that an `initialize` function on it is kept and called.

What this can't stop: the web view's speculative preloader may still **download** the file
(WebKit and Chromium fetch `<script src>` URLs ahead of the parser), so the vendor's page still
makes the web view contact Microsoft's CDN, as it does for its other resources; the file just
never runs. Answering those requests natively (WebView2's `WebResourceRequested`, WKWebView's
content rules) needs platform code with `unsafe`, which GridCraft's workspace forbids; it would
come through an audited crate, as `fmv-macos-events` does for Apple events.

## The bridge (`office.sync`)

The page's library sends `{"id": n, "actions": [...]}` through the web view; GridCraft answers
`{"id": n, "results": [...]}` or `{"id": n, "error": {"code", "message", "action"}}`. Each action
names an object by its **path** from the workbook (`{"p": property}`, `{"m": method, "a": args}`):

```json
{"op": "set",  "path": [{"p": "workbook"}, {"p": "worksheets"}, {"m": "getItem", "a": ["Data"]}, {"m": "getRange", "a": ["A1:B2"]}], "prop": "values", "value": [[1, 2], [3, 4]]}
{"op": "load", "path": [{"p": "workbook"}, {"m": "getSelectedRange"}], "props": ["values", "address"]}
{"op": "call", "path": [{"p": "workbook"}, {"m": "getSelectedRange"}], "method": "clear", "args": ["Formats"]}
```

Paths start at `workbook`, `document` (the common API's selection) or `settings`. Paths are
resolved again for every action, so a range is its address (as Excel's ranges are). Methods
with an effect (`worksheets.add`) are `call`s, never steps of a path; a worksheet added without
a name is found by the token its `add` call carried (`{"m": "getAdded", "a": [token]}`), which
follows renames made through the bridge. Writes go through
GridCraft's commands (`range.setValues`, `home.bold`, `sheet.rename`…), reads through `sheet.read`
and `cell.get`. Agents can send the same batches with the `office.sync` command.

Settings are kept per workbook and add-in while GridCraft runs; saving them in the file (the
web-extension parts of the XLSX package, [MS-OWEXML]) comes next.

## Security

- The add-in's page runs in the web view's sandbox, on its own `https://` origin; its access to
  the workbook is the bridge's, bounded by the manifest's permissions.
- The web view only navigates to `https://` pages; links that open a window go to the browser.
- GridCraft itself never contacts Microsoft: the add-in's page is loaded from its vendor, and the
  API is GridCraft's. A page that references Microsoft's `office.js` may still make the web view
  download it (above); it never runs.

## Testing with Microsoft's samples

`cargo xtask office-samples` runs Microsoft's official add-in samples
([OfficeDev/Office-Add-in-samples](https://github.com/OfficeDev/Office-Add-in-samples), MIT) in the
real desktop app: its task pane is the real web view (WKWebView, WebView2), and the pages load from
where Microsoft hosts them (`https://officedev.github.io/Office-Add-in-samples/…`). Nothing of the
samples is in GridCraft's repository: the manifests are read from a checkout made at test time (CI
clones it at a pinned commit), and the pages from GitHub Pages.

```sh
cargo build --release -p gridcraft && cargo xtask office-js
git clone --depth 1 https://github.com/OfficeDev/Office-Add-in-samples /tmp/office-samples
cargo xtask office-samples --app target/release/gridcraft --office-js target/office-js \
    --samples /tmp/office-samples --out /tmp/office-samples-out
```

For each scenario the driver starts a fresh app (`--control <port>`, a settings folder of its own
via `HOME`/`APPDATA`/`XDG_CONFIG_HOME`, `GRIDCRAFT_NO_PREFS`, a blank workbook), then through the
[control channel](control-protocol.md#office-add-ins-pages):

1. loads the sample's hosted manifest (`plugins.load`), opens its pane (`office.open`) and waits
   until its Office.js is ready (`office.panes`);
2. checks in the page (`office.eval`) that `Office.context.host` is `"Excel"`, that `Office` is
   GridCraft's (`__craftOfficeInfo.officeIsCraft`, set by the bootstrap) and that Microsoft's
   `office.js` never ran (`typeof OSF` is `"undefined"`; `__craftOfficeInfo.blockedScripts` lists
   the script URLs that got the empty script);
3. clicks the sample's buttons (`document.getElementById('run').click()`), runs its ribbon
   commands as the ribbon does (`plugins.run`, for ExecuteFunction), and checks the workbook
   (`cell.get`, `document.inspect`);
4. takes screenshots (`ui.screenshot`).

When something a check waits for doesn't happen, the driver asks the page for
`__craftOfficeInfo` and writes it to `diagnostics/<scenario>-<n>.json`; the report gets the gist.
The bootstrap keeps it from the start: uncaught errors and unhandled rejections, console errors
and warnings (still printed), each script that loaded or failed, the document's `readyState`,
the library's start error, and its `onReady` bookkeeping (`__craftOfficeState()`: calls, callbacks
run, the last error a callback threw, associated actions).

It writes `results.json`, `summary.md` (a table for `$GITHUB_STEP_SUMMARY`), the screenshots and the
app's logs into `--out`, and exits with 1 if a scenario failed. A failing scenario never stops the
others; an app that crashes or hangs is reported and the next scenario gets a fresh one.
`--only id,id` runs some, `--list` prints the table, `--dry-run` drives a fake app (no web view,
no network; the tests run it). The expectations are one table
(`xtask/src/office_samples/scenarios.rs`): a check either **must pass**, or is **expected
unsupported**, in which case it has to fail cleanly (an error the page can handle, or the add-in
blocked with the reason) and is reported as "works (expected unsupported)" if it starts working.

| Scenario | Sample | Must pass | Expected unsupported |
|---|---|---|---|
| excel-hello-world | `hello-world/excel-hello-world` | pane, GridCraft's Office, the button writes "Hello world!" into A1 | |
| excel-add-in-commands | `office-add-in-commands/excel` | the ribbon button's pane; Highlight fills the selection yellow; the menu's ExecuteFunction item writes its button id into the selection | |
| save-custom-settings | `office-add-in-save-custom-settings` | Save (`settings.saveAsync`); the value reads back in a reopened pane | |
| keyboard-shortcuts | `office-keyboard-shortcuts` | pane; the page starts; its AddNew and RunAction actions (run as a shortcut would) add a sheet and colour the selection | the shortcuts themselves (`replaceShortcuts`, ExtendedOverrides); `Office.addin.showAsTaskpane` |
| custom-functions-azure, -batching, -storage, -web-worker | `Excel-custom-functions/*` | (their pane, once they load) | loading: their manifests require SharedRuntime / CustomFunctionsRuntime, so they are blocked; custom functions |
| first-run-experience-tutorial | `tutorials/first-run-experience-tutorial` | | skipped: not hosted (`https://localhost:3000`) |

On GitHub, a fork-only workflow builds the app and the library on `macos-15` and
`windows-latest`, clones the samples at a pinned commit, runs the driver and uploads the
screenshots and results as an artifact, with the table in the job summary. Both runners have an
interactive desktop session, so the window and its web view are real (Windows renders with WARP,
the runner has no GPU).

## Limits and next steps

- **Platforms:** the desktop app on macOS and Windows. Linux and FreeBSD need WebKitGTK 4.1 in the
  packages; the web app needs an iframe host.
- **The bootstrap:** the API has to exist before the page's scripts run, and a wasm module of this
  size can't start synchronously in WebView2, so a short JavaScript file (`crates/office-js/bootstrap.js`)
  holds the first calls until the library is ready and keeps Microsoft's `office.js` from running
  ([above](#microsofts-officejs)). It is the one hand-written script in GridCraft.
- **Checked in the real web views by `cargo xtask office-samples` only** (macOS and Windows runners,
  [above](#testing-with-microsofts-samples)): the parser-timing part of the `office.js` blocking and
  the function commands' hidden web view; the workspace's tests run the bootstrap and the library
  in Node with a stand-in DOM.
- Not yet: Excel's object events (`onChanged`, `onSelectionChanged` on worksheets and tables;
  the document's `DocumentSelectionChanged` works), the dialog API, the shared runtime and
  `Office.addin`, keyboard shortcuts (`ExtendedOverrides`), `Office.ribbon`, custom functions (they need
  asynchronous calculation; see [gaps.md](gaps.md) #21 and the live-data decision), tables,
  charts, named items, conditional formats, the unified JSON manifest, AppSource, single sign-on
  with Microsoft accounts (`getAccessToken`), manifest icons.
- Installing an Office add-in from a manifest URL in Options › Add-ins.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-16 | minor | `Office.onReady` and `Office.initialize` wait for `DOMContentLoaded` (on macOS the library could start before a `<head>` script's page had its buttons); the bootstrap records what went wrong in the page for the samples driver's diagnostics |
| 2026-10-16 | minor | Testing with Microsoft's samples: `cargo xtask office-samples` drives the hosted samples in the real app's task pane (scenario table with must-pass and expected-unsupported checks, results.json and a summary table); the control channel's `office.panes`, `office.open`, `office.close`, `office.eval`; the bootstrap's read-only `__craftOfficeInfo` |
| 2026-10-15 | minor | Microsoft's samples: their `office.js` gets an empty script and can't replace the globals; colour names; function commands (ExecuteFunction) end to end; `context.host`, `new Excel.RequestContext()`, the common API's enumerations, `set({...})`, `worksheets.add()` without a name, `Office.initialize` assigned late, `DocumentSelectionChanged`, unsupported `*Async` methods answer `NotSupported`; AddinCommands 1.1 and DocumentEvents 1.1 reported; ExcelApi 1.4 described as the subset it is |
| 2026-10-15 | minor | `<Requirements>` (`Sets` with `MinVersion`/`DefaultMinVersion`, `Methods`) checked against the supported sets and methods: an unmet one blocks the add-in with the reason |
| 2026-10-14 | major | Created: Office add-in manifests as plugins, task panes in the system web view, GridCraft's Office.js, the `office.sync` bridge (ExcelApi 1.1–1.4 subset, common API selection and settings) |
