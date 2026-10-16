# Live sessions (co-editing)

> **Last reviewed:** 2026-10-16 · **Last updated:** 2026-10-16 · **Change:** minor (the web build edits live in Nextcloud through craft-live; `collab/snapshot` writes the asked `format`; previously minor: document chat, protocol 2.1; previously minor: selections on empty sheets and past the last row or column, clearer people circles, name tags; major: created: plugin protocol 2 in GridCraft, schema 1) · **Target:** Microsoft Excel (Microsoft 365)

Several people edit one workbook at the same time, each in their own GridCraft, and see each
other's selections and edits as they happen. This is GridCraft's equivalent of Excel's
co-authoring. It's **native**: GridCraft merges the edits itself (a CRDT, Yjs through the
[`yrs`](https://crates.io/crates/yrs) crate), shows who's there and keeps a per-person undo.
Moving the bytes between people is a **transport plugin**'s job (MQTT, a local relay, a
y-websocket server, Nextcloud…), which never needs to understand a workbook. With a room key the
content is encrypted end to end, so the transport only ever sees ciphertext.

The protocol is shared with WordCraft and DeckCraft:
[craft plugin protocol 2](https://github.com/sirmmo/craft-collab/blob/main/docs/protocol-2.md)
(push channel, rooms, `collab/*` messages, encryption, limits). This page is GridCraft's part:
the shared document (**schema 1**, normative) and how the app behaves. A summary of the schema
for other implementations is craft-collab's
[`docs/schemas/gridcraft-1.md`](https://github.com/sirmmo/craft-collab/blob/main/docs/schemas/gridcraft-1.md);
the two agree, and this page has the detail that one leaves to GridCraft.

## Using it

**Share**, at the right end of the title bar (as in Excel). Not in a session, it opens the
Share dialog: pick the transport add-in, then its own *Start a Live Session* or *Join from an
Invitation…* (the add-in makes and reads invitation links), or name a room and key under *Room
name and key*. With no transport add-in installed, the dialog says a live-session add-in is
needed, names craft-mqtt from [sirmmo/craft-collab](https://github.com/sirmmo/craft-collab) as an
example, and opens Automate › Add-ins. In a session, the button shows the session's state with a
dot (green: live; amber, ringed: connecting or joining; grey: offline, editing goes on here) and
opens the session's popover: the room's title, state, room id and transport, whether it's
encrypted or view-only, the transport's message, everyone in it (circle, name, "(you)"), *Copy
invitation* when the transport's Share or Join returned a link (`result.link`), and *Leave*
(the workbook stays open, with every edit, as an ordinary workbook; not saved automatically).

To the left of Share, the people in the session are lightly overlapping circles with their initials on
their colour (black or white text, whichever reads better), the others by name and then you,
at most four circles and then "+N" (hovering it lists the rest). Hover a circle for the name
("(you)" for yourself); click it to go to that person's selection, switching sheet if needed,
which flashes in their colour. In a narrow window the theme switch, Comments and Search give
way first, then circles; the document title is cut short with "…" before anything overlaps it.
In full screen on macOS the title bar's controls move into the traffic lights' space.

Others' selections are outlined in their colours, anywhere on the sheet (an empty sheet, or below
its last used row, too). Each has a name tag just above the top edge of the range holding their
active cell (in the column headers when that edge is at the top of the view); it fades out three
seconds after the person last moved and comes back while the pointer is over their outline, so
it doesn't hide the cells. Cells they change are highlighted for two seconds. The status bar only says why the workbook can't be
edited now ("Joining…", view only, another app's room) and the transport's or app's problems
("Wrong room key?"). The `collab.*` commands are in the command palette (they no longer have a
Review tab group).

Undo and Redo, while in a session, undo **your** changes only; others' edits never enter your
undo history. Saving works as usual: each person saves their own copy.

## Commands

| Command | Params | Does |
|---|---|---|
| `collab.share` | `{plugin, room, user?, role?, key?, title?}` | Starts a session from the active workbook (no params: the Share dialog) |
| `collab.join` | `{plugin, room, mode: share\|join, user?: {id, name, color}, role?: edit\|view, key?, path?, title?}` | Joins or starts a session (no params: the dialog). `share` seeds the room from the active workbook (or `path`); `join` opens a new workbook (or `path`, whose unshared parts are kept) filled from the room |
| `collab.leave` | `{room?}` | Leaves (default: the active workbook's room) |
| `collab.status` | `{}` | `{rooms: [{room, plugin, state, peers, listed, role, encrypted, clientId, user, title, document, message, canUndo, canRedo}], transports, user}` |
| `collab.setUser` | `{name?, color?}` | Your name (the same as File › Options › User name) and colour (`#RRGGBB`) |

A plugin command's result may carry `collab: {join: {…}}` (with protocol 1.2's `open`,
`dialog` and `message`): `plugins.run` returns it to the caller, and the interface runs
`collab.join` with it (the plugin's id as `plugin` unless given) when a button, menu item or
plugin dialog ran the command, keeping `result.link` as the room's invitation. Programmatic
calls never open dialogs.

Rust API (for embedders, tests and a future web build): `Session::collab_poll(now_ms)` runs the
sessions between frames; `Session::collab_serve_in_memory(id)` / `collab_join_in_memory`,
`collab_take_outgoing` and `collab_poll_pushed` connect a session to a transport the caller
implements; `collab_take_notices` gives the status-bar messages.

## Document chat

The people working on a workbook can talk in a chat beside it (craft-collab's
[`docs/chat.md`](https://github.com/sirmmo/craft-collab/blob/main/docs/chat.md), protocol 2.1,
the same in WordCraft and DeckCraft). The chat belongs to the workbook: it's shared live in a
session, saved in the file, and stays until someone removes it. Outside a session it works too,
as notes left in the file for whoever opens it next.

**Where.** The title bar's chat button, between Share and the people's circles (shown in a
session, or when the workbook has a chat), carries the number of unread messages from others; a
message that mentions you (`@Your Name`) while the pane is closed flashes it. It opens the Chat
pane, where the task panes show (also from View › Chat, or `view.chat`). Each message shows its
author's circle in their colour, name and time (relative today: "just now", "5 min ago",
"3 h ago"; else date and time), the text (selectable; links clickable; `@Name` highlighted when it
names someone in the session), the message it replies to quoted, and "edited" or "Message
deleted". Right-click a message: Reply, Copy Text, and for your own Edit and Delete. The box at the
bottom sends with Enter (Shift+Enter: a new line); "Name is typing…" shows above it. The pane's
menu: Export as Text, Export as JSON (copied to the clipboard), Clear chat… (asks "Remove the
whole chat from the document for everyone?"), and the note "Kept in the file; Office drops it when
it saves".

**Rules.** Messages are 1–4,000 characters (UTF-16 units); at most 10,000 ("The chat is full:
clear it to go on"; a deleted message still counts, Clear frees it). Only the author edits or
deletes a message; a deleted one stays as "Message deleted" so replies still point somewhere.
Chat changes are never undo steps and Undo/Redo keep the current chat, but they mark the
workbook modified (the chat is kept only when the file is saved); others' messages do too. A
viewer (`role: view`) can chat but not edit. While joining, the chat waits for the room.

**In the shared document.** The root array `chat` (one map per message: `id`, `author` as JSON
text `{id, name, color}`, `at`, `text`, `replyTo?`, `edited?`, `deleted?`) and the root map
`chatMeta` (`clearedAt`), outside the undo scope. GridCraft writes the change between the synced
workbook's chat and the new one in the command's transaction: new messages appended in order,
changed keys of edited or deleted ones, and for a clear every entry removed and `clearedAt` set.
Readers skip malformed entries and duplicate ids and keep the first 10,000. Sharing seeds the
chat from the workbook; joining replaces the workbook's chat with the room's. In an encrypted
room the chat is encrypted like every other update.

**In the file.** XLSX (and `.xlsm` and the templates): the part `/craft/chat.json` (content type
`application/vnd.craft.chat+json`, related from `_rels/.rels` with `urn:craft:relationships:chat`)
holding `{"version": 1, "clearedAt"?, "messages": […]}`; an empty chat writes no part; a damaged
part is skipped with a warning and the workbook opens without it. The native JSON format has the
same object as `chat`. Microsoft Excel doesn't know the part and drops it when it saves.

**Presence.** Your awareness state has `typing: true` while you're writing in the box.

| Command | Params | Does |
|---|---|---|
| `chat.list` | `{since?, limit?}` | `{messages, clearedAt?, count}`: the messages after the one with id `since` (or, for a number, sent after that time in ms), the newest `limit` of them |
| `chat.send` | `{text, replyTo?}` | `{id}` |
| `chat.edit` | `{id, text}` | Your own message only |
| `chat.delete` | `{id}` | Your own; it stays as "Message deleted" |
| `chat.clear` | `{}` | `{removed}`; the interface asks first, programmatic calls don't |
| `chat.export` | `{format?: text\|json, utcOffsetMinutes?}` | `{format, text}` (JSON also as `chat`) |
| `chat.typing` | `{typing?}` (alias `on`) | Tells the session you're typing (nothing outside one) |
| `view.chat` (interface) | `{on?}` | Shows or hides the Chat pane |

## In Nextcloud (craft-live)

[craft-live](https://github.com/sirmmo/craft-live) is a WOPI editor that Nextcloud Office uses in
place of Collabora Online. It opens spreadsheets in GridCraft's **web build**, framed in its shell
page at `index.html#craft-live=1&ws=…&room=…&token=…&user=…&role=…&seed=…&file=…&name=…&format=…&api=…&lang=…`.
In that mode the web build speaks craft-live's embed contract (craft-live `docs/design.md` §9)
natively, with no plugin: the WebSocket is the transport.

**What the person sees.** The Nextcloud file opens as the only workbook, titled with its name
(Nextcloud's language when the web build has it). The first person to open it loads it (a 0-byte
file is a new, empty workbook); the others see "Joining…" (read-only) until the room has it, and
so does the first person for the round trip in which the room takes it. Share in the title bar
shows the session's state; its popover says "Room <id> · Nextcloud", the file, who you are and
whether you can edit, whether everything is saved in Nextcloud ("All changes saved in Nextcloud",
"Unsaved changes: Nextcloud gets them automatically", "Saving…"), the connection ("Offline:
reconnecting in N s"), everyone in the room, and **Close** instead of Leave. The people's circles,
others' selections and edits, and the Chat pane work as in any live session. Nextcloud shows no
close button, so the title bar gets one (**Close**, at the right end), and File › Close and
Cmd/Ctrl+W close the document in Nextcloud too: with unsaved changes the room saves first
(waiting up to 10 s), then the editor closes. **Save** (Cmd/Ctrl+S, the title bar's Save, Save
As .xlsx) asks the room to save now; exports (PDF, CSV, web page) still download. A viewer
(`role=view`) reads along and chats, can't edit or save. When the session expires the workbook
becomes read-only and a message says to open the file again from Nextcloud.

**What the web build does** (`gridcraft_engine::live`, the browser I/O in
`apps/gridcraft-web/src/live.rs`):

- **Start.** Reads the fragment and posts `craft-live:hello` to the shell (same origin); a
  `craft-live:config` answer is used when the fragment can't be. It signs notes, comments and
  chat with the Nextcloud name.
- **Seeding** (`seed=1`): `GET <file>` with `Authorization: Bearer <token>`, opened as File ›
  Open does, then the room is seeded from it with `collab.share` semantics (origin `seed`).
  **Joining** (`seed=0`): `collab.join` semantics, read-only until the room's sync step 2.
- **The room.** `new WebSocket(<ws>?token=<token>)`, binary frames in y-websocket's lib0 framing
  (protocol 2, Appendix B): `0/0` sync step 1, `0/1` sync step 2, `0/2` update, `1` awareness,
  `2/0` permission denied, `3` query awareness (ignored), `100` craft-live control (JSON). It
  sends control `hello {clientId, can: ["upload"], client}` first, then its sync step 1; answers
  the room's sync step 1; sends one update per committed command and its awareness (selection,
  user, typing) at most 20 times a second. The engine's room is served in memory (transport id
  `craft-live`): exactly the protocol-2 core the desktop drives through `collab/*`.
- **Control messages.** `welcome` (a seeder whose lease went asks `POST <api>/prepare` again; one
  whose room already holds someone else's document joins it instead, so the content is never
  seeded twice; a joiner the room hands the lease to loads the file and seeds), `peers`,
  `modified`, `seeded` (the seeder can edit), `saved` / `saveFailed`, `notice` (status bar),
  `reseed` (`POST <api>/prepare`; with `{seed: true}` load the file, seed and reconnect),
  `expired` (read-only, a message). `save {id, mode: "upload", url, name, format}`: the workbook
  as a file in `format` (`xlsx`, `xlsm`, `xltx`, `xltm` as asked; anything else, `.ods` included,
  as an `.xlsx`, which craft-live keeps as a copy beside the file), `PUT <url>` with the bearer
  token, `X-Craft-File-Name` (percent-encoded) and the bytes; a failed upload is reported with
  `saveResult {id, ok: false, error}`. The token goes only to craft-live's own origin; `webdav`
  mode is refused (`saveResult`). Save sends `saveNow`; closing sends `bye`.
- **Close codes.** 4401 (expired) and 4403 (refused) end the session; anything else reconnects
  after 1 s, doubling up to 30 s, and syncs again (edits made offline merge).
- **To the shell:** `craft-live:loaded` (the document is shown), `modified`, `saved {success}`
  (after a save the person or Nextcloud asked for), `peers` (the others and yourself with
  `self: true`; also on `craft-live:getPeers`), `close`, `error` (it can't open the document).
  From the shell: `craft-live:save` (progress), `craft-live:close` (save if needed, leave),
  `craft-live:getPeers`; `craft-live:host` is ignored.

**The desktop app** joins the same rooms through craft-live's nextcloud-live add-in (a protocol-2
transport). craft-live asks it for the file with `collab/snapshot {room, file: true, format}`:
GridCraft writes `format` when it can (`xlsx`, `xlsm`, `xltx`, `xltm`) and otherwise an `.xlsx`,
and says which in the answer's `format`.

Building the bundle craft-live serves: `packaging/web/package.sh` (the site in `dist/web/`),
then point craft-live's `apps.gridcraft` at that folder ([packaging/web/README.md](../packaging/web/README.md#inside-nextcloud-craft-live)).

## Schema 1 (normative)

`meta.app` is `"gridcraft"`, `meta.schema` is `1` (written as an integer). All binary encodings
are Yjs v1 updates.

### Roots

| Root | Yjs type | Holds |
|---|---|---|
| `meta` | Map | `app`, `schema`, `title` (string; the shared workbook's title, `""` for an unsaved one) |
| `sheets` | Array | sheet ids, in tab order |
| `sheet:<id>` | Map | one sheet (below) |
| `names` | Map | defined names: key = the name in lower case, plus `@<sheet index>` for a sheet-local name; value = JSON text `{name, scope, formula, comment, hidden}` (`formula` without `=`, `scope` a sheet index or `null`) |
| `styles` | Map | cell styles: key = `s` + the first 16 hex digits of the SHA-256 of the style's JSON; value = that JSON text (GridCraft's `Style`: `font`, `fill`, `borders`, `align`, `num_fmt`, `protection`) |
| `book` | Map | workbook settings, each JSON text, last writer wins: `normal` (the Normal style, which unstyled cells use), `cellStyles`, `customLists`, `theme`, `dateSystem`, `calc`, `props`, `protectedStructure`, `structurePassword` (a hash, never a password) |

Styles are content-addressed: two people creating the same style write the same entry. A cell
whose style hasn't arrived yet shows unstyled until it does.

### A sheet: `sheet:<id>`

| Key | Type | Holds |
|---|---|---|
| `name` | string | the tab's name |
| `rows`, `cols` | Array | row and column ids, in order (entry `i` is row `i + 1`, column `i + 1`) |
| `rowBlocks`, `colBlocks` | number | the next unused block number (below) |
| `cells` | Map | `"<rowId>:<colId>"` → Map `{input: string, style?: string, array?: string}` |
| `rowProps`, `colProps` | Map | row or column id → JSON text `{size?, hidden?, outline?, collapsed?, custom?, style?}` (`size` in points, `style` a key of `styles`) |
| `merges` | Map | top-left `"<rowId>:<colId>"` → bottom-right `"<rowId>:<colId>"` |
| `tables`, `charts`, `cf`, `dv` | string | JSON text of the sheet's tables, charts, conditional formats and data validations (GridCraft's model), last writer wins |
| `props` | Map | other sheet settings, each JSON text, last writer wins: `defaultColWidth`, `defaultRowHeight`, `freeze`, `tabColor`, `visibility`, `showGridlines`, `showHeadings`, `showZeros`, `rightToLeft`, `autofilter`, `images`, `shapes`, `sparklines`, `slicers`, `pivots`, `protection`, `editRanges`, `print`, `comments`, `hyperlinks`, `cellPictures` (the last three as `[[A1, value], …]`) |

Unknown keys are kept and ignored. Positions inside the JSON features (`tables`, `charts`,
`comments`…) are A1 addresses, not ids: the peer that inserts rows rewrites them with its
update, and a concurrent change of the same feature keeps the later write.

### Ids

- **Sheet ids and the ids of inserted rows and columns**: 8 random characters of `0-9a-z`.
- **Growing past the end**: rows and columns beyond the arrays exist implicitly and are empty.
  When an edit (or the seed) needs one, the writer appends whole **blocks of 64** deterministic
  ids, block `b` being `~b.0` … `~b.63`, numbered from the larger of `rowBlocks`/`colBlocks` and
  one more than the highest block present, then sets the counter to the next unused number. Two
  people growing a sheet at once append the same ids, which collapse when read, so their cells
  land in the same rows. A seed grows each sheet to its used range (cells, row and column
  properties, merges): a 4-row sheet has 64 row ids, an empty one none.
- **Duplicates** (concurrent moves, concurrent growth): readers keep the first occurrence of an
  id in `sheets`, `rows` and `cols`; positions count after that. A sheet id without a
  `sheet:<id>` map (or with an empty one) is skipped.

### Cells

`input` is what the person typed, decoded exactly (no locale, no number format):

| Input | Value |
|---|---|
| empty | no value (a cell with only a style) |
| `=…` | a formula (its canonical text after `=`) |
| `'…` | text, when the text alone would read as something else (`'007`, `'=A1`, `'TRUE`) |
| `TRUE`, `FALSE` | logical |
| `#N/A`, `#DIV/0!`… (Excel's spelling) | an error value |
| decimal notation Rust's `f64` parser reads, finite (`3.5`, `-2`, `1e-7`) | a number, written in shortest round-trip form |
| anything else | text |

`style` is a key of `styles`; without it the cell uses the Normal style. `array` marks a legacy
array formula (Ctrl+Shift+Enter): the bottom-right cell of its range. Setting a cell that has a
Map updates `input` (and `style`) inside it, so a concurrent style change survives; clearing
it deletes the key. Keys whose row or column id isn't in `rows`/`cols` are ignored.

### Edits

Each committed command (the engine's undo step, outermost: a plugin command's steps are one) is
**one** transaction with GridCraft's local origin and one `collab/update`. GridCraft computes it
by comparing the workbook before and after the command:

| Edit | Transaction |
|---|---|
| Cells | set or remove the changed keys (only input, style and array count: recalculated values never travel) |
| Insert rows/columns before `p` | new random ids into the array before the `p`-th id (nothing when `p` is past the end); the cells move with their ids; the inputs Excel would rewrite (`=SUM(A1:A10)` → `=SUM(A1:A11)`) are rewritten in the same transaction |
| Delete rows/columns | delete the cells in them, remove every occurrence of their ids, rewrite shifted inputs |
| Move columns | an insert at the destination and a delete at the source |
| Add, rename, delete, move sheets | an id into `sheets` with its `sheet:<id>`; `name`; the id out of `sheets` (the map stays, so undo brings it back); out and in at the new place |
| Formats, row heights, merges, features | the changed entries |

A peer applying a remote change maps rows and columns by id and **doesn't** rewrite formulas
again. Commands that change only view state (selection, zoom, scroll, the active sheet) send
nothing; the selection goes to awareness.

### Awareness

Each peer's state is `{user: {id, name, color}, selection?, cursor?}`:
`selection` = `{sheet: <sheetId>, ranges: [[rowId, colId, rowId, colId], …]}` (top-left and
bottom-right cells), `cursor` = `{sheet, cell: [rowId, colId]}` (the active cell). Rows and
columns past the last id are named by the block id they would get if the sheet grew to them now
(`~<n + k div 64>.<k mod 64>` for the `k`-th position past the end, `n` the next block number);
nothing is written to the document for it. A reader resolves a block id it doesn't have yet to
that position, so selections on an empty sheet, or below the last row, show; when the sheet grows,
the same ids name the same cells (schema 1, "Awareness"). GridCraft sends its state at most 20 times a second
and renews it every 15 s; a peer not heard from for 30 s is gone.

## Behaviour

- **Push channel.** Every running plugin's standard output is read all the time; what a plugin
  sends on its own (notifications, `collab/snapshot` and `host/log` requests) waits in a queue
  that the app's main loop takes each frame, never in the middle of a command. `host/…` requests
  during one of GridCraft's requests are still answered by it (protocol 1.1). Lines may be
  64 MiB; a plugin sending more than 1,000 messages a second for 5 s is stopped and marked failed.
- **Joining** (`mode: join`): `collab/joined`, then `collab/sync1`; the workbook is empty and
  read-only until a state for GridCraft schema 1 arrives. **Sharing**: the room is seeded from
  the workbook (origin `seed`, not undoable), then `collab/joined` and `collab/sync1`.
- **Sync.** A received `sync1` is answered with `sync2` addressed to the asker (`to` = its
  `from`); when its state vector shows changes GridCraft lacks, GridCraft asks back with its own
  `sync1`. `sync1` also goes out when the transport reports `online` after another state. An
  update that skips some of a peer's changes waits until they arrive (and GridCraft asks for
  them): yrs would otherwise apply it without reporting all its changes.
- **Remote changes** are applied between frames and recalculated (the changed cells, or
  everything after a structural change). A message that can't be decoded or applied is dropped
  and shown in the status bar; an update that writes another app's `meta` is dropped and says
  "This room is for <app> <schema>". Nothing a peer sends can crash GridCraft (panics in the
  Yjs library are caught too).
- **Encryption** (a `key`, 32 bytes): AES-256-GCM per protocol 2 section 4; `sync2` with
  `snapshot: true` and the answer to `collab/snapshot` use the method name `collab/snapshot`.
  After 10 messages that don't decrypt, the status bar asks "Wrong room key?".
- **`collab/snapshot`** is answered with the whole state (encrypted when keyed) and, with
  `file: true`, the workbook's file bytes with `name` and `format`: in `format` when the request
  names one GridCraft writes (`xlsx`, `xlsm`, `xltx`, `xltm`), else the workbook's own Excel kind
  or `.xlsx` (an `.ods` comes back as `.xlsx`, `format: "xlsx"`). In an encrypted room GridCraft
  answers `fileRefused` instead, so the plaintext never reaches the transport.
- **Offline.** When the transport reports `offline` or exits, editing goes on; an exited
  transport is started again after 3 s (a crash counts as a plugin failure) and the room rejoins
  with `sync1`, which merges both sides' edits.
- **Recovery.** Every 30 s the desktop app writes each room's whole state into
  `<settings folder>/recovery/room-<hash>.json`; joining that room again (`mode: join`) merges
  it first, so offline edits made before a crash aren't lost. Leaving deletes the file.
- **Viewers** (`role: view`) can't edit; anything that changes their workbook goes back to the
  room's state.
- **Limits.** An update is at most 16 MiB before base64 (a larger local change is announced with
  `sync1` so peers fetch it; a larger remote one is refused), awareness 64 KiB, 16 rooms per
  transport, one room per workbook.

## Tests

- `crates/collab/src/tests.rs`: two and three replicas with random concurrent edits (cells,
  styles, rows, columns, sheets, merges) and updates delivered out of order and twice converge,
  equal what a newcomer builds, and each model equals its own document; per-user undo; another
  app's room refused; garbage never crashes; selections on an empty sheet and past the last
row or column (whole columns too) travel as block ids, write nothing, and name the same cells
after the sheet grows. `GRIDCRAFT_COLLAB_SEEDS` soaks it.
- `crates/engine/tests/collab_convergence.rs`: two and three sessions run random command
  sequences (cell edits, formats, row and column inserts and deletes, merges, sheets, per-user
  undo and redo) with updates exchanged in random order and duplicated; they end with identical
  workbooks and calculated values.
- `crates/engine/tests/collab_conformance.rs`: replays craft-collab's GridCraft schema-1
  transcripts (`CRAFT_COLLAB_DIR`, or a `craft-collab` checkout next to this one).
- `crates/engine/tests/collab_push.rs`: a fake transport (the test binary) relays between two
  sessions through the push channel: joining from a plugin command's result, sync, edits both
  ways, `collab/snapshot` with the file, awareness (a selection below the last row and one on a
new, empty sheet reach the other session), undo, leaving.
- Chat: `crates/model/src/chat.rs` (the rules, limits, the file part's JSON),
  `crates/xlsx/src/tests/chat_tests.rs` (the part's round trip, an empty chat writing none, a
  damaged part skipped), `crates/collab/src/tests.rs` (three replicas chatting at once converge;
  clear while someone sends; undo leaves the chat), `crates/engine/tests/chat.rs` (the commands
  and their limits, undo, modified, XLSX and JSON; in sessions: two and three people converging,
  clearing, a newcomer, an encrypted room, a viewer, typing), chat sends and clears in
  `collab_convergence.rs`, craft-collab's `gridcraft-1-chat*` transcripts in
  `collab_conformance.rs`, and `crates/ui-egui/tests/chat_pane.rs` (the unread count, the pane,
  Enter and Shift+Enter, the mention flash, Clear chat… asking, the button outside a session).
- `crates/engine/tests/collab_loopback.rs`: two GridCraft processes through craft-collab's
  loopback transport (`CRAFT_COLLAB_LOOPBACK` = the built plugin).
- craft-live: `crates/engine/src/live/tests.rs` runs the client against a fake craft-live room
  (a plain Yjs document, as the server holds): framing round trips and damaged frames, the
  fragment as craft-live writes it, seeding, a joiner waiting, converging both ways, one update
  per command, Save reaching the room as a workbook GridCraft reads back, the seeder read-only
  until the room has the seed, reseeding after the seeder left, a seeder whose lease was taken
  joining instead (the room's document untouched), close codes, backoff and offline edits
  merging, `expired`, saving in the file's format (`.xltx`, `.xlsm`; `.ods` as `.xlsx`), a refused
  upload, a foreign upload address and WebDAV mode refused, viewers, closing after the save (or
  10 s), File › Close, files that can't be had, hostile messages, and the desktop's
  `collab/snapshot {format}`. `crates/engine/tests/live_wasm.rs` runs the client compiled to
  wasm (wasm-bindgen-test-runner; random client ids from the browser's crypto, seeding, the
  upload). `crates/ui-egui/tests/host_session.rs`: the title bar's Close, File › Close through
  the host, the popover. `apps/gridcraft-web/e2e` (not a workspace member; needs a craft-live
  checkout next to this one): the client against the real craft-live server and its fake
  Nextcloud with proof keys: the editor page's fragment, seeding from the WOPI file, a second
  person converging, Save reaching PutFile as a workbook GridCraft reads back, a viewer, closing
  after the save, an `.ods` saved as an `.xlsx` copy beside it.

## Not yet

- Text inside a cell is a plain string, last writer wins (two people typing in one cell keep the
  later input), not a Yjs text.
- Tables, charts, conditional formats, validation, comments and other features are shared as
  whole JSON values per sheet: two people changing the same feature at once keep the later.
- The web build has no plugins; it speaks the y-websocket binding natively only inside
  craft-live (no Share dialog to start or join other rooms from the browser).
- craft-live mode, verified only against craft-live's fake Nextcloud and a native WebSocket so
  far: a real browser (the `postMessage` handshake with the shell, `fetch` and the WebSocket from
  the iframe, the Close button) and a real Nextcloud (craft-live `docs/design.md` §12) are still
  to check.
- yrs garbage collection is off (it crashed with the undo manager in the fuzzer): deleted
  content stays in the document's history.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-16 | minor | In Nextcloud (craft-live): the web build speaks craft-live's embed contract (fragment and shell messages, seeding from the file or joining, y-websocket over `web_sys::WebSocket`, control messages, uploading the file in its own format, reconnecting, Close in the title bar, the session's popover); the desktop's `collab/snapshot {file, format}` writes the asked format; the web build's random numbers come from the browser's crypto |
| 2026-10-16 | minor | Document chat (protocol 2.1, craft-collab `docs/chat.md`): the `chat`/`chatMeta` roots, the XLSX part `/craft/chat.json` and the native format's `chat`, the `chat.*` commands and `view.chat`, awareness `typing`, the Chat pane and the title bar's chat button with the unread count and the mention flash; replays the chat transcripts |
| 2026-10-15 | minor | Selections on a sheet with no row or column ids, and past the last ones, travel as the block ids those rows and columns would get (no document write; schema 1 clarified in craft-collab); people circles slightly larger, with less overlap and clearer initials; name tags sit above the selection's top edge (in the column headers at the top of the view), fade after 3 s without movement and come back on hover over the outline |
| 2026-10-15 | minor | Share and the session's people in the title bar (state dot, popover with people, Copy invitation and Leave; circles with "+N"; click to go to a person's selection); the Share dialog points at Automate › Add-ins when no transport is installed; a transport command's `collab.join` is now joined by the interface and its invitation kept; Review › Share group removed; status bar keeps only read-only reasons and problems |
| 2026-10-15 | major | Created: plugin protocol 2 (2.0.1) in GridCraft: push channel, rooms, schema 1, encryption, per-user undo, awareness, recovery, Review › Share |
