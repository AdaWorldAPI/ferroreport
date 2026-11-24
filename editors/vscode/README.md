# ferroreport templates for VS Code

Write [ferroreport](https://github.com/cristianEncarnacion/ferroreport) report templates
(`.ferro`) with syntax highlighting, snippets and folding, and see the PDF change each time you
save.

![Typing an "if" block with a snippet in invoices.ferro, saving, and the PDF beside it updating with the new red "Overdue" lines](https://raw.githubusercontent.com/cristianEncarnacion/ferroreport/main/editors/vscode/images/demo.gif)

## What you get

- **Highlighting** for every part of a template: sections (`title`, `detail`, `row`...),
  parts (`textField`, `staticText`, `image`...), declarations (`field`, `parameter`,
  `variable`), sizes like `2fr`, attributes, and the expression after ` : `, with its
  `$F{}`, `$P{}` and `$V{}` references, strings and numbers.
- **Snippets** that type the structure for you:

  | Type | You get |
  |---|---|
  | `report` | A new report with a style, a field, a title and a detail |
  | `row` | Parts side by side |
  | `if` | Parts printed only when a condition holds |
  | `each` | Parts repeated for every row of a nested list |
  | `def` / `use` | A definition, and a place that uses it again |
  | `import` | Definitions and styles from another file |

- **Folding** by indentation, comments with `Ctrl+/`, and automatic indentation after a line
  that opens a section.

## See the PDF while you edit

The extension edits the template; the [ferroreport](https://github.com/cristianEncarnacion/ferroreport)
command renders it. Run this in the terminal and keep the PDF open beside the template: it is
written again every time you save the template or the data.

```bash
ferroreport render invoices.ferro -d invoices.json -o invoices.pdf --watch
```

Change the extension of `-o` to get `.xlsx`, `.docx`, `.csv` or `.html` from the same
template. To get the command, download it from
[Releases](https://github.com/cristianEncarnacion/ferroreport/releases) or run
`cargo install ferroreport-cli`.

![A .ferro invoice template with highlighting next to the PDF it renders](https://raw.githubusercontent.com/cristianEncarnacion/ferroreport/main/editors/vscode/images/screenshot.png)

## Learn the format

- [The .ferro format](https://cristianencarnacion.github.io/ferroreport/docs.html#ferro): every
  section, part and attribute.
- [Examples](https://github.com/cristianEncarnacion/ferroreport/tree/main/examples): the
  invoice from the animation (`invoices.ferro`, `parts.ferro`, `invoices.json`) and more.
- Already have JasperReports templates? `ferroreport` renders `.jrxml` files as they are.

## License

MIT or Apache-2.0, at your option.
