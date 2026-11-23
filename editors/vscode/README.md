# ferroreport templates for VS Code

Syntax highlighting, folding by indentation and snippets (`report`, `row`, `if`, `each`, `def`,
`use`, `import`) for [ferroreport](https://github.com/cristianEncarnacion/ferroreport)'s `.ferro`
report templates: kinds of parts, positions and sizes, attributes, and the expressions after
` : ` with their `$F{}`, `$P{}`, `$V{}` references and JasperReports' functions.

Render while you edit with `ferroreport render invoice.ferro -d rows.json -o invoice.pdf --watch`
and keep the PDF open beside the template: it is written again each time you save.

To install it from this folder: `npx @vscode/vsce package`, then *Extensions → … → Install from
VSIX*.
