//! Table components. JasperReports fills a `jr:table` as a subreport over the table's
//! subdataset, built from the columns that print (`TableReport` in JasperReports 6.20):
//!
//! | table row        | report band                                                    |
//! |------------------|----------------------------------------------------------------|
//! | table header     | title                                                          |
//! | column headers   | column header (printed again on every page the table spans)    |
//! | detail cells     | detail                                                         |
//! | column footers   | page footer                                                    |
//! | table footer     | summary                                                        |
//! | group header/footer cells | the subdataset's group header/footer                  |
//!
//! Headers are laid out top-down, a column group's cell above its columns; footers bottom-up,
//! a column group's cell below its columns.

use crate::error::Result;
use crate::expr::Expr;
use crate::model::*;

/// A column that prints, with its printed width (a column group is as wide as its printed
/// columns).
pub struct FillColumn<'a> {
    pub column: &'a TableColumn,
    pub width: f32,
    pub children: Vec<FillColumn<'a>>,
}

/// The columns that print, given a way to evaluate `printWhenExpression`s.
pub fn fill_columns<'a>(
    columns: &'a [TableColumn],
    prints: &mut dyn FnMut(&Expr) -> Result<bool>,
) -> Result<Vec<FillColumn<'a>>> {
    let mut out = Vec::new();
    for column in columns {
        if let Some(e) = &column.print_when {
            if !prints(e)? {
                continue;
            }
        }
        if column.is_group {
            let children = fill_columns(&column.children, prints)?;
            if children.is_empty() {
                continue;
            }
            let width = children.iter().map(|c| c.width).sum();
            out.push(FillColumn { column, width, children });
        } else {
            out.push(FillColumn { column, width: column.width, children: Vec::new() });
        }
    }
    Ok(out)
}

/// The cell element placed at (x, y), `width` wide. When a column group prints narrower than
/// designed, its cell's children are scaled like Jasper's `scaleCellElement`.
fn place(cell: &TableCell, designed_width: f32, width: f32, x: f32, y: f32) -> Element {
    let mut el = cell.element.clone();
    el.x = x;
    el.y = y;
    el.width = width;
    if width != designed_width && designed_width > 0.0 {
        let scale = |e: &mut Element| {
            e.x = (e.x * width / designed_width).floor();
            e.width = (e.width * width / designed_width).floor();
        };
        if let ElementKind::Frame { children, .. } = &mut el.kind {
            for child in children.iter_mut() {
                scale(child);
                if let ElementKind::Frame { children: grand, .. } = &mut child.kind {
                    grand.iter_mut().for_each(scale);
                }
            }
        }
    }
    el
}

type CellOf<'c> = dyn Fn(&TableColumn) -> Option<&TableCell> + 'c;

/// Headers and detail: a group's cell, then its columns below it.
fn forward(col: &FillColumn, cell_of: &CellOf, x: &mut f32, y: f32, out: &mut Vec<Element>) {
    if col.column.is_group {
        let mut sub_y = y;
        if let Some(cell) = cell_of(col.column) {
            out.push(place(cell, col.column.width, col.width, *x, y));
            sub_y += cell.height;
        }
        for sub in &col.children {
            forward(sub, cell_of, x, sub_y, out);
        }
    } else {
        if let Some(cell) = cell_of(col.column) {
            out.push(place(cell, col.column.width, col.width, *x, y));
        }
        *x += col.column.width;
    }
}

/// Footers: a group's columns first, then the group's cell below the tallest of them.
/// Returns the y below what was placed.
fn reverse(col: &FillColumn, cell_of: &CellOf, x: &mut f32, y: f32, out: &mut Vec<Element>) -> f32 {
    if col.column.is_group {
        let start_x = *x;
        let mut bottom = y;
        for sub in &col.children {
            bottom = bottom.max(reverse(sub, cell_of, x, y, out));
        }
        if let Some(cell) = cell_of(col.column) {
            out.push(place(cell, col.column.width, col.width, start_x, bottom));
            bottom += cell.height;
        }
        bottom
    } else {
        let mut bottom = y;
        if let Some(cell) = cell_of(col.column) {
            out.push(place(cell, col.column.width, col.width, *x, y));
            bottom += cell.height;
        }
        *x += col.column.width;
        bottom
    }
}

/// The band for one table row, or `None` when no printed column has a cell in it (or the
/// cells have no height), as in Jasper.
/// A band of the table from one cell of each column. `tag` is what those cells are in a
/// tagged PDF: header cells or data cells.
fn band(columns: &[FillColumn], cell_of: &CellOf, bottom_up: bool, tag: Tag) -> Option<Band> {
    let mut elements = Vec::new();
    let mut x = 0.0;
    for col in columns {
        if bottom_up {
            reverse(col, cell_of, &mut x, 0.0, &mut elements);
        } else {
            forward(col, cell_of, &mut x, 0.0, &mut elements);
        }
    }
    let height = elements.iter().map(|e| e.y + e.height).fold(0.0, f32::max);
    if height <= 0.0 {
        return None;
    }
    for e in &mut elements {
        e.tag = e.tag.or(Some(tag));
    }
    Some(Band { height, split_type: SplitType::Prevent, print_when: None, elements })
}

fn group_cell<'c>(cells: &'c [(String, TableCell)], group: &str) -> Option<&'c TableCell> {
    cells.iter().find(|(n, _)| n == group).map(|(_, cell)| cell)
}

/// The report JasperReports fills for a table whose printed columns are `columns`, placed in
/// an element `width` points wide.
pub fn report(table: &TableElement, columns: &[FillColumn], width: f32) -> Report {
    let dataset = table.run.dataset.clone().unwrap_or(Dataset {
        name: "table".into(),
        parameters: Vec::new(),
        fields: Vec::new(),
        variables: Vec::new(),
        groups: Vec::new(),
    });
    let detail = band(columns, &|c| c.detail.as_ref(), false, Tag::DataCell);
    let column_header = band(columns, &|c| c.column_header.as_ref(), false, Tag::HeaderCell);
    let title = band(columns, &|c| c.table_header.as_ref(), false, Tag::HeaderCell);
    let summary = band(columns, &|c| c.table_footer.as_ref(), true, Tag::DataCell);
    let page_footer = band(columns, &|c| c.column_footer.as_ref(), true, Tag::DataCell);

    let mut groups = dataset.groups;
    for g in &mut groups {
        let name = g.name.clone();
        g.header = band(columns, &|c| group_cell(&c.group_headers, &name), false, Tag::HeaderCell).into_iter().collect();
        g.footer = band(columns, &|c| group_cell(&c.group_footers, &name), true, Tag::DataCell).into_iter().collect();
    }

    // With both column footers and a table footer, the last column footers go in the footer of
    // an outermost group (so they print before the table footer) and the last page has an
    // empty page footer.
    let mut last_page_footer = None;
    if let (Some(pf), Some(_)) = (&page_footer, &summary) {
        groups.insert(
            0,
            Group {
                name: "__TableSummary".into(),
                expression: None,
                start_new_page: false,
                start_new_column: false,
                reprint_header_on_each_page: false,
                min_height_to_start_new_page: 0.0,
                header: Vec::new(),
                footer: vec![pf.clone()],
            },
        );
        last_page_footer = Some(Band { height: 0.0, split_type: SplitType::Prevent, print_when: None, elements: Vec::new() });
    }

    let no_data = table.no_data.as_ref().map(|cell| Band {
        height: cell.height,
        split_type: SplitType::Prevent,
        print_when: None,
        elements: vec![place(cell, width, width, 0.0, 0.0)],
    });

    Report {
        name: dataset.name,
        resource_bundle: None,
        when_resource_missing: WhenResourceMissing::Null,
        language: table.run.language,
        page_width: width,
        page_height: table.run.page_height,
        landscape: false,
        column_width: width,
        column_count: 1,
        column_spacing: 0.0,
        margins: Margins { left: 0.0, right: 0.0, top: 0.0, bottom: 0.0 },
        when_no_data: table.when_no_data,
        title_new_page: false,
        summary_new_page: false,
        summary_with_page_header_and_footer: false,
        float_column_footer: false,
        parameters: dataset.parameters,
        fields: dataset.fields,
        variables: dataset.variables,
        groups,
        background: None,
        title,
        page_header: None,
        column_header,
        detail: detail.into_iter().collect(),
        column_footer: None,
        page_footer,
        last_page_footer,
        summary,
        no_data,
        datasets: Vec::new(),
        uses_data_source: true,
        horizontal: false,
        pdf_page_offsets: Default::default(),
        warnings: Vec::new(),
    }
}

/// The report JasperReports effectively fills for a list: the contents as the detail band,
/// nothing else; a horizontal list fills them across as many columns as fit.
pub fn list_report(list: &ListElement, width: f32) -> Report {
    let (columns, column_width) = match list.across {
        Some(w) => (((width / w).floor() as u32).max(1), w),
        None => (1, width),
    };
    let run = &list.run;
    let dataset = run.dataset.clone().unwrap_or(Dataset {
        name: "list".into(),
        parameters: Vec::new(),
        fields: Vec::new(),
        variables: Vec::new(),
        groups: Vec::new(),
    });
    Report {
        name: dataset.name,
        resource_bundle: None,
        when_resource_missing: WhenResourceMissing::Null,
        language: run.language,
        page_width: width,
        page_height: run.page_height,
        landscape: false,
        column_width,
        column_count: columns,
        column_spacing: 0.0,
        margins: Margins { left: 0.0, right: 0.0, top: 0.0, bottom: 0.0 },
        when_no_data: WhenNoData::NoPages,
        title_new_page: false,
        summary_new_page: false,
        summary_with_page_header_and_footer: false,
        float_column_footer: false,
        parameters: dataset.parameters,
        fields: dataset.fields,
        variables: dataset.variables,
        groups: dataset.groups.into_iter().map(|g| Group { header: Vec::new(), footer: Vec::new(), ..g }).collect(),
        background: None,
        title: None,
        page_header: None,
        column_header: None,
        detail: vec![list.contents.clone()],
        column_footer: None,
        page_footer: None,
        last_page_footer: None,
        summary: None,
        no_data: None,
        datasets: Vec::new(),
        uses_data_source: true,
        horizontal: columns > 1,
        pdf_page_offsets: Default::default(),
        warnings: Vec::new(),
    }
}
