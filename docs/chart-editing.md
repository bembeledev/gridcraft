# Chart editing and analysis

Contribution based on v0.11.1, updated 2026-10-23.

## Interface

Ribbon tabs have a horizontal viewport and a More tabs menu in narrow windows. A separate
overflow menu exposes the active tab's registered commands. Dialogs can be moved and resized;
long forms scroll. The Data Analysis chooser reserves its footer outside the scrolling list.
Analysis input and output fields have range-picking buttons for the active worksheet.

## Chart editing

- Select chart elements with the mouse or Current Selection. Series selection precedes point
  selection; individual point formatting is not yet implemented.
- Format chart and plot fills/borders; primary-axis bounds, units, logarithmic scaling, reverse
  order and number formats; major/minor tick marks and axis/gridline strokes.
- Format series line/fill, transparency, widths, smoothing, marker type/size/fill/border,
  column gap width and overlap. Legend symbols follow series marker styling.
- Use Home font controls for chart titles, axis labels/titles, legend, data labels and trendline
  labels. Selecting a chart keeps these commands directed at chart text.
- Place the legend inside the plot and drag its box or the trendline equation label.
- Use chart context menus to select data, change chart type, add elements, duplicate, format,
  copy, or export. PNG export offers scale factors; SVG retains vector geometry and text.
- Save named chart templates in GridCraft's local UI preferences, apply one to a chart of the same
  type, or mark one as the default for newly inserted charts of that type. Templates currently stay
  on this installation; they are not embedded in a workbook and `.crtx` import/export is not yet
  supported.
- Clipboard copies carry an image for other apps. Editable duplication within the current
  workbook uses the saved chart snapshot only while the clipboard image still matches it.

Supported settings survive GridCraft XLSX save/reopen. Common properties are written as native
chart XML; a GridCraft extension preserves the complete settings. Cross-editor changes to native
properties can leave that extension stale. Excel/LibreOffice interoperability is not certified.

## Regression

The Analysis ToolPak offers linear, exponential, logarithmic and power models. Nonlinear fits
use transformed variables; coefficients and inferential statistics belong to that transformed
domain. They are not original-scale nonlinear least-squares diagnostics. Positive inputs and
single-X restrictions are validated where the selected transformation requires them. Regression
inputs and the first-row-label option are grouped at the top, with model and fit options separated;
OK and Cancel stay fixed below the scrollable form.

## Follow-up work

Verified on 2026-10-23 with workspace type checking, 399 engine unit tests, 9 chart-command
integration tests, 52 chart tests, 253 XLSX tests, 157 UI unit tests and 55 selected UI interaction
tests. Responsive tests exercise hidden ribbon tabs, a 480-pixel-high analysis chooser, a fixed
regression footer while scrolling, and an actual worksheet drag into a regression range field.
Offscreen snapshots were inspected for the analysis chooser, chart format pane and the full
regression form at 900×800. Desktop clipboard exchange with external applications and
Excel/LibreOffice interoperability still need manual verification.

- Error bars: X/Y direction, fixed/percentage/error/deviation/custom ranges, caps, numerical
  validation, renderer and XLSX persistence.
- Complete data-label content, cell references, positioning, callouts, leader lines, rotation,
  wrapping, margins and number formats; independent point/label formatting.
- Axis crossing, display units, configurable log base, date/category axes and full secondary axes.
- Minor gridlines and separate major/minor tick strokes/lengths; additional dash patterns,
  compound lines, caps, joins and arrows.
- Per-point and negative-value fills; gradients, textures, patterns, shadows and other effects.
- Individual trendline editing/removal, forecast/intercept/name controls and equation number formats.
- Data tables, drop/high-low/series lines, up/down bars, portable `.crtx` templates and editable chart sheets.
- Hit-testing for overlapping or same-colored series; pie wedges and legend entries; selection
  outlines after geometry edits; keyboard and drag undo/redo across all chart kinds.
- Clipboard replacement, cut, cross-document and web-platform behavior; SVG font embedding and
  consistency of screen, PNG, SVG, HTML and PDF typography.
- Native XLSX formatting coverage, stale extension resolution and preservation of unsupported XML.
- Further regression diagnostics, LOGEST/GROWTH compatibility and optional additional models.
- 3-D rendering and chart kinds requiring new infrastructure remain deferred.
