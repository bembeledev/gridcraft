# Formula language

View → Interface language is the existing, persisted language preference. Choose **Español**
for Spanish or **Türkçe** for Turkish formula entry and display. On first run the preference
follows the system language. English and the other interface languages currently use the English
formula table and separators.

| Spanish | Canonical English |
| --- | --- |
| `SUMA` | `SUM` |
| `PROMEDIO` | `AVERAGE` |
| `SI` | `IF` |
| `CONTAR` | `COUNT` |
| `SUMAR.SI` | `SUMIF` |
| `CONTAR.SI` | `COUNTIF` |
| `BUSCARV` | `VLOOKUP` |
| `HOY` | `TODAY` |
| `VERDADERO`, `FALSO` | `TRUE`, `FALSE` (literals or function calls) |

Names are case-insensitive. Spanish formulas use decimal commas (`1,5`), semicolon arguments
and reference unions, and backslashes between array columns (`{1\2;3\4}`). For example,
`=SI(VERDADERO;SUMA(1,5;2,25);0)` evaluates to 3.75. English mode retains comma arguments,
decimal points and comma array columns; it does not silently translate Spanish aliases.

The cell editor, formula bar, Show Formulas, autocomplete, the Formula ribbon, Insert Function, and argument hints
use the selected formula language. Common Spanish function help is translated; functions
without a Spanish entry retain their English names/help. General interface labels and errors
fall back to English where Spanish translations are not provided. An edit captures its language
when it starts so switching the preference cannot reinterpret unfinished input.

## Turkish

Turkish uses Microsoft Excel's Turkish function names for 439 functions
(`crates/formula/src/turkish.rs`): `TOPLA`, `EĞER`, `DÜŞEYARA`, `ETOPLA`, `EĞERSAY`,
`BAĞ_DEĞ_SAY`, `İNDİS`, `KAÇINCI`, `DEVRESEL_ÖDEME`, `ÇAPRAZARA`… and `DOĞRU` / `YANLIŞ` for
TRUE and FALSE. Names follow Turkish case rules: `eğer` is `EĞER`, `indis` is `İNDİS` (`i` ↔ `İ`,
`ı` ↔ `I`). Arguments are separated by `;`, decimals use a comma, and array constants separate
columns with `;` and rows with `\`, as in Turkish Excel: `=TOPLA({1;2\3;4};1,5)` is 11.5.
Functions Turkish Excel leaves in English (`LET`, `LAMBDA`, `BYROW`, `MOD`, `RANK`…) and those
whose Turkish name is not confirmed yet keep their English names; English names are always
accepted. Excel's own Turkish names are kept even where they look swapped (`IMSUM` is
`SANÇIKAR`, `IMSUB` is `SANTOPLA`), so a formula typed as in Turkish Excel means the same here.

Workbook/sheet names, table names, LET bindings and LAMBDA parameters take precedence over
aliases. Text, sheet names and table column names are unchanged. When a localized spelling
would collide with a name, displayed formulas retain the canonical spelling to preserve meaning.

Stored formulas, XLSX import/export, engine commands, control/API calls, CLI evaluation and
clipboard exchange retain strict English syntax. This also applies to advanced dialogs such as
Name Manager and conditional-format rules. Localization happens at the cell/formula-bar editor
boundary, including filling the selected range with Ctrl+Enter; it does not change the workbook
format or teach the calculation engine ambiguous aliases.
