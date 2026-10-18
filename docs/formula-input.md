# Formula language

View → Interface language is the existing, persisted language preference. Choose **Español** for
Spanish formula entry and display, or **Русский** for Russian. On first run the preference follows
the system language. English and the other interface languages currently use the English formula
table and separators.

| Spanish | Russian | Canonical English |
| --- | --- | --- |
| `SUMA` | `СУММ` | `SUM` |
| `PROMEDIO` | `СРЗНАЧ` | `AVERAGE` |
| `SI` | `ЕСЛИ` | `IF` |
| `CONTAR` | `СЧЁТ` | `COUNT` |
| `SUMAR.SI` | `СУММЕСЛИ` | `SUMIF` |
| `CONTAR.SI` | `СЧЁТЕСЛИ` | `COUNTIF` |
| `BUSCARV` | `ВПР` | `VLOOKUP` |
| `HOY` | `СЕГОДНЯ` | `TODAY` |
| `VERDADERO`, `FALSO` | `ИСТИНА`, `ЛОЖЬ` | `TRUE`, `FALSE` (literals or function calls) |

The Spanish table is small. The Russian table covers about 200 worksheet functions (math,
statistics, logical, text, date, lookup, information and financial; see
`crates/formula/src/input/ru.rs`). Functions without an entry keep their English name in every
language. `LET` and `LAMBDA` keep their English names.

Names are case-insensitive, and in Russian `е` matches `ё` (`СЧЕТ` and `СЧЁТ` are the same name).
Spanish and Russian formulas use decimal commas (`1,5`), semicolon arguments and reference unions,
and backslashes between array columns (`{1\2;3\4}`). For example, `=SI(VERDADERO;SUMA(1,5;2,25);0)`
and `=ЕСЛИ(ИСТИНА;СУММ(1,5;2,25);0)` both evaluate to 3.75. English mode retains comma arguments,
decimal points and comma array columns; it does not silently translate Spanish or Russian aliases.

The cell editor, formula bar, Show Formulas, autocomplete, the Formula ribbon, Insert Function, and argument hints
use the selected formula language. Common Spanish and Russian function help is translated; functions
without a translated entry retain their English names/help. General interface labels and errors
fall back to English where translations are not provided. An edit captures its language
when it starts so switching the preference cannot reinterpret unfinished input.

Workbook/sheet names, table names, LET bindings and LAMBDA parameters take precedence over
aliases. Text, sheet names and table column names are unchanged. When a localized spelling
would collide with a name, displayed formulas retain the canonical spelling to preserve meaning.

Stored formulas, XLSX import/export, engine commands, control/API calls, CLI evaluation and
clipboard exchange retain strict English syntax. This also applies to advanced dialogs such as
Name Manager and conditional-format rules. Localization happens at the cell/formula-bar editor
boundary, including filling the selected range with Ctrl+Enter; it does not change the workbook
format or teach the calculation engine ambiguous aliases.
