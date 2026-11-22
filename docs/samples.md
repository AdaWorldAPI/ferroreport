# JasperReports' own samples, rendered by ferroreport

The 140 templates of the `demo/samples` folder of JasperReports 6.20.6, rendered by both engines from
the same rows and parameters and compared page by page. Made by `tools/samples.py`; nothing here is written by hand.

| Result | Templates | Meaning |
|---|---|---|
| same | 14 | every page is the same picture |
| differs | 54 | both engines render it, the pictures differ |
| not rendered | 19 | ferroreport refuses the template or fails on it |
| no reference | 53 | JasperReports cannot render it from rows alone (it needs the sample's Java classes, a database connection inside the report, or other files), so there is nothing to compare with |

**14 of the 87 templates that can be compared come out the same.** 21 of them
name no font for some of their text: JasperReports then draws it with whatever fonts the Java runtime has, which no
other engine can reproduce (and neither can another machine). Of the 66 that name their fonts,
14 come out the same.

These samples were written to show every feature of JasperReports, one per sample: charts of every kind, maps, forms,
Java scriptlets, database connections inside the report. They are a much harder test than the reports people usually
write, which is what `fixtures/` holds (all pixel-identical). Use `ferroreport doctor your.jrxml` to see where yours stands.

| Template | Result | Pages | Why |
|---|---|---|---|
| `accessible/reports/AccessibleReport.jrxml` | no reference |  | needs Java classes that are not at hand (the sample's own, or libraries it adds); ferroreport renders it |
| `alterdesign/reports/AlterDesignReport.jrxml` | same | 1/1 |  |
| `antcompile/reports/com/bar/BarReport.jrxml` | not rendered |  | error evaluating expression: unsupported static method SomeBarClass.isToPrint() |
| `antcompile/reports/com/foo/FooReport.jrxml` | not rendered |  | error evaluating expression: unsupported static method SomeFooClass.isToPrint() |
| `antupdate/reports/com/bar/BarReport.jrxml` | not rendered |  | error evaluating expression: unsupported static method SomeBarClass.isToPrint() |
| `antupdate/reports/com/foo/FooReport.jrxml` | not rendered |  | error evaluating expression: unsupported static method SomeFooClass.isToPrint() |
| `barbecue/reports/BarbecueReport.jrxml` | differs | 1/1 | worst page differs in 95.4% of its ink: names no font, so JasperReports draws it with the JVM's own; <barbecue> elements are not supported yet and are skipped |
| `barcode4j/reports/Barcode4JReport.jrxml` | differs | 1/1 | worst page differs in 42.5% of its ink: names no font, so JasperReports draws it with the JVM's own; <DataMatrix> elements are not supported yet and are skipped; <EAN128> elements are not supported yet and are skipped; <USPSIntelligentMail> elements are not supported yet and are skipped |
| `batchexport/reports/Report1.jrxml` | differs | 2/2 | worst page differs in 14.4% of its ink: names no font, so JasperReports draws it with the JVM's own |
| `batchexport/reports/Report2.jrxml` | differs | 2/2 | worst page differs in 42.3% of its ink: names no font, so JasperReports draws it with the JVM's own |
| `batchexport/reports/Report3.jrxml` | differs | 2/2 | worst page differs in 49.1% of its ink: names no font, so JasperReports draws it with the JVM's own |
| `book/reports/BookReport.jrxml` | differs | 1/1 | worst page differs in 99.3% of its ink |
| `book/reports/OrdersReport.jrxml` | not rendered |  | error evaluating expression: variable `ROW_COUNT` is not a measure or group of the crosstab |
| `book/reports/PieChartReport.jrxml` | no reference |  | JasperReports draws nothing for it without the sample's own program; ferroreport renders it |
| `book/reports/TOCPart.jrxml` | same | 1/1 |  |
| `chartcustomizers/reports/ChartCustomizersReport.jrxml` | no reference |  | needs Java classes that are not at hand (the sample's own, or libraries it adds); ferroreport renders it |
| `charts/reports/AreaChartReport.jrxml` | differs | 13/13 | worst page differs in 3.9% of its ink |
| `charts/reports/Bar3DChartReport.jrxml` | differs | 13/13 | worst page differs in 7.4% of its ink |
| `charts/reports/BarChartReport.jrxml` | differs | 13/13 | worst page differs in 8.5% of its ink: chart customizers (customizerClass) are ignored |
| `charts/reports/BubbleChartReport.jrxml` | differs | 19/19 | worst page differs in 23.8% of its ink: <bubbleChart> elements are not supported yet and are skipped |
| `charts/reports/CandlestickChartReport.jrxml` | differs | 18/18 | worst page differs in 60.8% of its ink: <candlestickChart> elements are not supported yet and are skipped |
| `charts/reports/GanttChartReport.jrxml` | differs | 1/1 | worst page differs in 32.7% of its ink: <ganttChart> elements are not supported yet and are skipped |
| `charts/reports/HighLowChartReport.jrxml` | differs | 18/18 | worst page differs in 9.0% of its ink: <highLowChart> elements are not supported yet and are skipped |
| `charts/reports/LineChartReport.jrxml` | differs | 13/13 | worst page differs in 3.8% of its ink |
| `charts/reports/MeterChartReport.jrxml` | differs | 16/16 | worst page differs in 51.7% of its ink: <meterChart> elements are not supported yet and are skipped |
| `charts/reports/MultipleAxisChartReport.jrxml` | differs | 24/24 | worst page differs in 87.3% of its ink: <multiAxisChart> elements are not supported yet and are skipped |
| `charts/reports/Pie3DChartReport.jrxml` | differs | 13/13 | worst page differs in 10.8% of its ink |
| `charts/reports/PieChartReport.jrxml` | differs | 13/13 | worst page differs in 9.3% of its ink |
| `charts/reports/ScatterChartReport.jrxml` | differs | 19/19 | worst page differs in 16.8% of its ink |
| `charts/reports/StackedAreaChartReport.jrxml` | differs | 13/13 | worst page differs in 18.4% of its ink |
| `charts/reports/StackedBar3DChartReport.jrxml` | differs | 13/13 | worst page differs in 6.9% of its ink |
| `charts/reports/StackedBarChartReport.jrxml` | differs | 13/13 | worst page differs in 3.3% of its ink |
| `charts/reports/SubDatasetChartReport.jrxml` | differs | 7/7 | worst page differs in 28.2% of its ink: names no font, so JasperReports draws it with the JVM's own; <timeSeriesChart> elements are not supported yet and are skipped |
| `charts/reports/ThermometerChartReport.jrxml` | differs | 16/16 | worst page differs in 18.0% of its ink: <thermometerChart> elements are not supported yet and are skipped |
| `charts/reports/TimeSeriesChartReport.jrxml` | differs | 19/19 | worst page differs in 15.0% of its ink: <timeSeriesChart> elements are not supported yet and are skipped |
| `charts/reports/XYAreaChartReport.jrxml` | differs | 19/19 | worst page differs in 17.8% of its ink |
| `charts/reports/XYBarChartReport.jrxml` | differs | 19/19 | worst page differs in 36.4% of its ink: <xyBarChart> elements are not supported yet and are skipped |
| `charts/reports/XYBarChartTimePeriodReport.jrxml` | differs | 19/19 | worst page differs in 15.0% of its ink: <xyBarChart> elements are not supported yet and are skipped |
| `charts/reports/XYBarChartTimeSeriesReport.jrxml` | differs | 19/19 | worst page differs in 22.3% of its ink: <xyBarChart> elements are not supported yet and are skipped |
| `charts/reports/XYLineChartReport.jrxml` | differs | 19/19 | worst page differs in 12.8% of its ink |
| `chartthemes/reports/AllChartsReport.jrxml` | differs | 10/10 | worst page differs in 97.0% of its ink: chart datasets run on a subdataset (datasetRun) are empty; <xyBarChart> elements are not supported yet and are skipped; chart datasets run on a subdataset (datasetRun) are empty |
| `crosstabs/reports/LateOrdersReport.jrxml` | not rendered |  | error evaluating expression: NullPointerException: cannot call toString() on null |
| `crosstabs/reports/OrdersReport.jrxml` | not rendered |  | error evaluating expression: variable `ROW_COUNT` is not a measure or group of the crosstab |
| `crosstabs/reports/ProductsReport.jrxml` | differs | 3/3 | worst page differs in 6.4% of its ink |
| `crosstabs/reports/ShipmentsReport.jrxml` | same | 2/2 |  |
| `csvdatasource/reports/CsvDataSourceReport.jrxml` | no reference |  | JasperReports draws nothing for it without the sample's own program; ferroreport renders it |
| `csvdatasource/reports/CsvQueryExecuterReport.jrxml` | no reference |  | JasperReports draws nothing for it without the sample's own program; ferroreport renders it |
| `customvisualization/reports/D3Rectangle.jrxml` | no reference |  | needs files that the sample's program provides; ferroreport renders it |
| `customvisualization/reports/Figures.jrxml` | no reference |  | needs files that the sample's program provides; ferroreport renders it |
| `customvisualization/reports/LeafletMarkers.jrxml` | no reference |  | needs files that the sample's program provides; ferroreport renders it |
| `customvisualization/reports/RadialProgress.jrxml` | no reference |  | needs files that the sample's program provides; ferroreport renders it |
| `customvisualization/reports/SimpleRectangle.jrxml` | no reference |  | needs files that the sample's program provides; ferroreport renders it |
| `customvisualization/reports/Sparkline.jrxml` | no reference |  | needs files that the sample's program provides; ferroreport renders it |
| `datasource/reports/DataSourceReport.jrxml` | same | 1/1 |  |
| `daterange/reports/DateRangeReport.jrxml` | not rendered |  | cannot parse expression `DATERANGE("1996-09-01")`: unknown identifier `DATERANGE` |
| `ejbql/reports/JRMDbCastSubreport.jrxml` | same | 1/1 |  |
| `ejbql/reports/JRMDbReport.jrxml` | no reference |  | JasperReports draws nothing for it without the sample's own program; ferroreport renders it |
| `ejbql/reports/JRMDbVariaSubreport.jrxml` | no reference |  | JasperReports draws nothing for it without the sample's own program; ferroreport renders it |
| `exceldataadapter/reports/ExcelXlsDataAdapterReport.jrxml` | no reference |  | needs Java classes that are not at hand (the sample's own, or libraries it adds); ferroreport renders it |
| `exceldataadapter/reports/ExcelXlsQeDataAdapterReport.jrxml` | no reference |  | needs Java classes that are not at hand (the sample's own, or libraries it adds); ferroreport renders it |
| `exceldataadapter/reports/ExcelXlsxDataAdapterReport.jrxml` | no reference |  | needs Java classes that are not at hand (the sample's own, or libraries it adds); ferroreport renders it |
| `exceldataadapter/reports/ExcelXlsxQeDataAdapterReport.jrxml` | no reference |  | needs Java classes that are not at hand (the sample's own, or libraries it adds); ferroreport renders it |
| `exceldatasource/reports/ExcelDataSourceReport.jrxml` | no reference |  | JasperReports draws nothing for it without the sample's own program; ferroreport renders it |
| `fastexceldatasource/reports/FastExcelDataSourceReport.jrxml` | no reference |  | JasperReports draws nothing for it without the sample's own program; ferroreport renders it |
| `fonts/reports/FontsReport.jrxml` | no reference |  | needs files that the sample's program provides; ferroreport renders it |
| `forms/reports/FormsReport.jrxml` | differs | 1/1 | worst page differs in 45.7% of its ink: names no font, so JasperReports draws it with the JVM's own |
| `functions/reports/FunctionsReport.jrxml` | no reference |  | needs files that the sample's program provides; ferroreport renders it |
| `genericelement/reports/GenericElementReport.jrxml` | differs | 1/1 | worst page differs in 86.1% of its ink: names no font, so JasperReports draws it with the JVM's own; <genericElement> elements are not supported yet and are skipped |
| `groovy/reports/GroovyReport.jrxml` | no reference |  | JasperReports fails on it without the sample's own program; ferroreport: cannot parse expression `[5, 6, 7, 8, 9].findAll{it % 2 == 1}`: unexpected character `[` |
| `hibernate/reports/AddressesReport.jrxml` | differs | 1/1 | worst page differs in 26.0% of its ink |
| `hibernate/reports/DocumentsReport.jrxml` | same | 1/1 |  |
| `hibernate/reports/HibernateQueryReport.jrxml` | differs | 1/1 | worst page differs in 10.3% of its ink |
| `horizontal/reports/HorizontalReport.jrxml` | differs | 14/20 | worst page differs in 100.0% of its ink: printOrder="Horizontal" is rendered as "Vertical" |
| `htmlcomponent/reports/HtmlComponentReport.jrxml` | no reference |  | JasperReports fails on it without the sample's own program; ferroreport renders it |
| `httpdataadapter/reports/JsonHttpDataAdapterReport.jrxml` | no reference |  | needs Java classes that are not at hand (the sample's own, or libraries it adds); ferroreport: error evaluating expression: NullPointerException: arithmetic on a null value |
| `httpdataadapter/reports/XmlHttpDataAdapterReport.jrxml` | no reference |  | its rows come from a xPath query; ferroreport renders it |
| `hyperlink/reports/HyperlinkReport.jrxml` | not rendered |  | unsupported feature: image format (only PNG and JPEG are supported) |
| `i18n/reports/I18nReport.jrxml` | not rendered |  | unsupported feature: image format (only PNG and JPEG are supported) |
| `iconlabel/reports/IconLabelReport.jrxml` | differs | 2/2 | worst page differs in 71.3% of its ink: <iconLabel> elements are not supported yet and are skipped |
| `images/reports/ImagesReport.jrxml` | not rendered |  | unsupported feature: image format (only PNG and JPEG are supported) |
| `jasper/reports/FirstJasper.jrxml` | differs | 9/9 | worst page differs in 0.7% of its ink |
| `java1.5/reports/Java5Report.jrxml` | not rendered |  | error evaluating expression: unsupported static field Greeting.bye |
| `javascript/reports/JavaScriptReport.jrxml` | differs | 1/1 | worst page differs in 72.2% of its ink: names no font, so JasperReports draws it with the JVM's own |
| `jfreechart/reports/JFreeChartReport.jrxml` | differs | 1/1 | worst page differs in 34.1% of its ink: names no font, so JasperReports draws it with the JVM's own |
| `jsondatasource/reports/JsonCustomersReport.jrxml` | no reference |  | JasperReports draws nothing for it without the sample's own program; ferroreport renders it |
| `jsondatasource/reports/JsonOrdersReport.jrxml` | no reference |  | JasperReports draws nothing for it without the sample's own program; ferroreport renders it |
| `jsonqldatasource/reports/NorthwindOrdersReport.jrxml` | no reference |  | its rows come from a jsonql query; ferroreport renders it |
| `landscape/reports/LandscapeReport.jrxml` | differs | 1/1 | worst page differs in 4.1% of its ink: names no font, so JasperReports draws it with the JVM's own |
| `list/reports/HorizontalListReport.jrxml` | same | 2/2 |  |
| `list/reports/ListReport.jrxml` | same | 1/1 |  |
| `map/reports/MapReport.jrxml` | no reference |  | needs files that the sample's program provides; ferroreport: error evaluating expression: parameter `REPORT_CONTEXT` is not declared |
| `map/reports/MapReport1.jrxml` | no reference |  | JasperReports fails on it without the sample's own program; ferroreport: error evaluating expression: parameter `REPORT_CONTEXT` is not declared |
| `map/reports/MapReport2.jrxml` | no reference |  | needs files that the sample's program provides; ferroreport: error evaluating expression: parameter `REPORT_CONTEXT` is not declared |
| `map/reports/MapReport3.jrxml` | no reference |  | needs files that the sample's program provides; ferroreport: error evaluating expression: parameter `REPORT_CONTEXT` is not declared |
| `map/reports/MapReport4.jrxml` | no reference |  | needs files that the sample's program provides; ferroreport: error evaluating expression: parameter `REPORT_CONTEXT` is not declared |
| `map/reports/MapReport5.jrxml` | no reference |  | needs files that the sample's program provides; ferroreport: error evaluating expression: parameter `REPORT_CONTEXT` is not declared |
| `map/reports/MapReport6.jrxml` | no reference |  | needs files that the sample's program provides; ferroreport: error evaluating expression: parameter `REPORT_CONTEXT` is not declared |
| `map/reports/MapReport7.jrxml` | no reference |  | needs files that the sample's program provides; ferroreport: error evaluating expression: parameter `REPORT_CONTEXT` is not declared |
| `markup/reports/MarkupReport.jrxml` | differs | 1/1 | worst page differs in 0.2% of its ink: markup="rtf" is rendered as plain text |
| `mondrian/reports/AdventureWorks-Olap4jXMLA-XJoinColumnsReport.jrxml` | no reference |  | JasperReports draws nothing for it without the sample's own program; ferroreport renders it |
| `mondrian/reports/AdventureWorks-Olap4jXMLAReport.jrxml` | no reference |  | JasperReports draws nothing for it without the sample's own program; ferroreport renders it |
| `mondrian/reports/AdventureWorks-XMLA-XJoinColumnsReport.jrxml` | no reference |  | JasperReports draws nothing for it without the sample's own program; ferroreport renders it |
| `mondrian/reports/AdventureWorks-XMLAReport.jrxml` | no reference |  | JasperReports draws nothing for it without the sample's own program; ferroreport renders it |
| `mondrian/reports/FoodMartSales-XMLAReport.jrxml` | no reference |  | JasperReports draws nothing for it without the sample's own program; ferroreport renders it |
| `mondrian/reports/FoodMartSalesWeekly-XMLAReport.jrxml` | no reference |  | JasperReports draws nothing for it without the sample's own program; ferroreport renders it |
| `mondrian/reports/MondrianFoodMartSalesOlap4jReport.jrxml` | no reference |  | JasperReports draws nothing for it without the sample's own program; ferroreport renders it |
| `mondrian/reports/MondrianFoodMartSalesReport.jrxml` | no reference |  | JasperReports draws nothing for it without the sample's own program; ferroreport renders it |
| `mondrian/reports/MondrianReport.jrxml` | no reference |  | JasperReports draws nothing for it without the sample's own program; ferroreport renders it |
| `nopagebreak/reports/NoPageBreakReport.jrxml` | not rendered |  | error evaluating expression: unsupported constructor new ArrayList() |
| `paragraphs/reports/ParagraphsReport.jrxml` | differs | 1/1 | worst page differs in 84.9% of its ink |
| `pdfencrypt/reports/PdfEncryptReport.jrxml` | differs | 1/1 | worst page differs in 86.8% of its ink: names no font, so JasperReports draws it with the JVM's own |
| `query/reports/QueryReport.jrxml` | not rendered |  | unsupported feature: image format (only PNG and JPEG are supported) |
| `rotation/reports/RotationReport.jrxml` | differs | 1/1 | worst page differs in 1.3% of its ink |
| `scriptlet/reports/ScriptletReport.jrxml` | not rendered |  | error evaluating expression: NullPointerException: cannot call hello() on null |
| `shapes/reports/ShapesReport.jrxml` | differs | 1/1 | worst page differs in 50.1% of its ink: names no font, so JasperReports draws it with the JVM's own |
| `spiderchartcomponent/reports/SpiderChart.jrxml` | no reference |  | JasperReports fails on it without the sample's own program; ferroreport renders it |
| `stretch/reports/StretchReport.jrxml` | differs | 21/14 | worst page differs in 100.0% of its ink: names no font, so JasperReports draws it with the JVM's own |
| `styledtext/reports/StyledTextReport.jrxml` | differs | 1/1 | worst page differs in 2.8% of its ink |
| `subreport/reports/AddressReport.jrxml` | no reference |  | JasperReports draws nothing for it without the sample's own program; ferroreport renders it |
| `subreport/reports/MasterReport.jrxml` | differs | 1/1 | worst page differs in 58.9% of its ink: subreports with a connectionExpression get no rows (only data sources are supported) |
| `subreport/reports/ProductReport.jrxml` | same | 1/1 |  |
| `table/reports/TableReport.jrxml` | same | 1/1 |  |
| `tableofcontents/reports/TableOfContentsReport.jrxml` | differs | 1/2 | worst page differs in 100.0% of its ink: a different number of pages |
| `tableofcontents/reports/TablePart.jrxml` | not rendered |  | error evaluating expression: variable `MASTER_CURRENT_PAGE` is not declared |
| `tableofcontents/reports/TocPart.jrxml` | same | 1/1 |  |
| `tabular/reports/TabularReport.jrxml` | same | 2/2 |  |
| `templates/reports/StylesReport.jrxml` | differs | 1/1 | worst page differs in 44.7% of its ink: names no font, so JasperReports draws it with the JVM's own; <template> is not supported yet; unknown style `Emphasis`; unknown style `Special Emphasis` |
| `text/reports/TextReport.jrxml` | same | 2/2 |  |
| `unicode/reports/UnicodeReport.jrxml` | differs | 2/2 | worst page differs in 95.8% of its ink: names no font, so JasperReports draws it with the JVM's own |
| `virtualizer/reports/VirtualizerReport.jrxml` | not rendered |  | error evaluating expression: unsupported static method Math.log() |
| `webapp/web/reports/WebappReport.jrxml` | no reference |  | JasperReports fails on it without the sample's own program; ferroreport: cannot parse expression `((scriptlets.WebappScriptlet)$P{REPORT_SCRIPTLET}).hello()`: expected RParen, found Param("REPORT_SCRIPTLET") |
| `xchart/reports/XChartReport.jrxml` | no reference |  | needs Java classes that are not at hand (the sample's own, or libraries it adds); ferroreport renders it |
| `xchartcomponent/reports/XYChart.jrxml` | no reference |  | JasperReports fails on it without the sample's own program; ferroreport renders it |
| `xlsfeatures/reports/ExcelNames.jrxml` | no reference |  | needs Java classes that are not at hand (the sample's own, or libraries it adds); ferroreport renders it |
| `xlsfeatures/reports/FormatPatternsMap.jrxml` | same | 1/1 |  |
| `xlsfeatures/reports/MacroReport.jrxml` | not rendered |  | error evaluating expression: NullPointerException: cannot call substring() on null |
| `xlsfeatures/reports/XlsFeaturesReport.jrxml` | not rendered |  | error evaluating expression: NullPointerException: cannot call substring() on null |
| `xlsformula/reports/XlsFormulaReport.jrxml` | differs | 1/1 | worst page differs in 41.2% of its ink: names no font, so JasperReports draws it with the JVM's own |
| `xmldatasource/reports/CustomersReport.jrxml` | no reference |  | JasperReports draws nothing for it without the sample's own program; ferroreport renders it |
| `xmldatasource/reports/OrdersReport.jrxml` | no reference |  | JasperReports draws nothing for it without the sample's own program; ferroreport renders it |
