# gridcraft-audit

Audit a `.xlsx` file for hidden information before sharing. Reads the raw OPC
package (ZIP) bytes — never the document model — and reports what a recipient
of the file could learn: hidden rows, columns and sheets (the data stays in
the file), external workbook links, author names on comments, creator /
last-modified-by in document properties, pivot caches that can embed source
data, macro/embedded-object parts, and package parts nothing references.

The scan reads raw package bytes, never the `gridcraft-xlsx` document model:
the model drops unmodeled parts on round-trip, so a model-based audit could
miss exactly the things being looked for.

## What it detects

| ID | Severity | What |
|---|---|---|
| `hidden/row` | warn | `row` with `hidden` set in a worksheet — the cells stay in the file |
| `hidden/column` | warn | `col` with `hidden` set — the cells stay in the file |
| `hidden/sheet` | warn | `sheet` with `state="hidden"` or `"veryHidden"` in `xl/workbook.xml` |
| `hidden/defined-name` | info | `definedName` with `hidden` set in `xl/workbook.xml` |
| `external-link` | warn | `externalReference` in `xl/workbook.xml` / parts under `xl/externalLinks/` |
| `author-pii/comment` | warn | author name on a legacy comment (`authorId` → `authors` list) |
| `author-pii/threaded-comment` | warn | `displayName` on a threaded comment, resolved via `xl/persons/` |
| `author-pii/core-properties` | warn | `dc:creator` / `cp:lastModifiedBy` in `docProps/core.xml` |
| `author-pii/extended-properties` | info | `Company` / `Manager` in `docProps/app.xml` |
| `pivot-cache` | warn | parts under `xl/pivotCache/` — the cache can embed source data |
| `active-content/vba` | warn | an `xl/vbaProject.bin` part (macros) |
| `active-content/ole` | warn | parts under `xl/embeddings/` (embedded objects) |
| `package/orphan-part` | info | ZIP entries no relationship targets |

Finding IDs are the crate's public vocabulary: they are stable and must not be
renamed (add a new variant instead).

Matching is by namespace URI, not by prefix, so producer-chosen prefixes do
not evade the scan. `hidden` counts when its value is true-ish (`1`, `true`,
`on`, `yes`); absent or false-ish is not flagged. A sheet counts as hidden
for `state="hidden"` or `state="veryHidden"` (ECMA-376 §18.18.68
`ST_SheetState`). Threaded-comment elements are accepted in the main
SpreadsheetML namespace and the 2018 threaded-comments namespace, since
producers vary.

The audit is visibility only: it never modifies the file, and active content
is reported, never blocked. Sanitizing is a separate, future step.

## Report shape

`gridcraft-cli audit <file>` prints:

```json
{
  "file": "budget.xlsx",
  "findings": [
    {
      "id": "hidden/sheet",
      "title": "hidden worksheet",
      "severity": "warn",
      "location": "xl/workbook.xml <sheet name=\"Salaries\">",
      "detail": "state=\"veryHidden\""
    }
  ],
  "count": 1
}
```

Exit codes: `0` = no findings, `1` = findings present (CI-gate usable),
`2` = the audit could not run (unreadable file, unparsable package).

## Spec references

- ECMA-376 Part 1: `row` (§18.3.1.73), `col` (§18.3.1.17), `sheet`
  (§18.2.28), `ST_SheetState` (§18.18.68), `definedName` (§18.2.6),
  `externalReference` (§18.2.8), comments (§18.7.3), threaded comments
  (§18.7.6)
- ECMA-376 Part 2: core properties (§12.3.1), extended properties (§12.3.2),
  OPC relationships (§12.2.4)

## Limits

- Hidden rows/columns are reported per `row`/`col` element; a `col` spanning
  a range reports the A1 letter range (e.g. `hidden columns B:D`).
- Threaded-comment authors resolve through `xl/persons/`; an unresolvable
  `personId` is reported as unresolved rather than dropped.
- A part that cannot be parsed is an error, never a silent pass (fail closed).
- Hostile-input guards: at most 100,000 ZIP entries, 256 MiB per part,
  2,000,000 XML elements per part.
