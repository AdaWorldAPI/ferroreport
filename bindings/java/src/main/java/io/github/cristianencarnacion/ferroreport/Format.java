package io.github.cristianencarnacion.ferroreport;

/** What a report is rendered to. */
public enum Format {
    PDF("pdf", "application/pdf"),
    /** An Excel workbook, laid out like JasperReports' XLSX exporter. */
    XLSX("xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
    /** Comma-separated values, laid out like JasperReports' CSV exporter. */
    CSV("csv", "text/csv; charset=utf-8"),
    /** A web page that looks like the PDF, for previews. */
    HTML("html", "text/html; charset=utf-8"),
    /** A Word document, laid out like JasperReports' DOCX exporter. */
    DOCX("docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document");

    private final String id;
    private final String mediaType;

    Format(String id, String mediaType) {
        this.id = id;
        this.mediaType = mediaType;
    }

    /** The name the engine knows the format by: {@code pdf}, {@code xlsx}, {@code csv}, {@code html}, {@code docx}. */
    public String id() {
        return id;
    }

    /** The media type to send the document with ({@code Content-Type}). */
    public String mediaType() {
        return mediaType;
    }

    /** The format a file name asks for by its extension; PDF when it does not say. */
    public static Format of(String fileName) {
        String lower = fileName.toLowerCase(java.util.Locale.ROOT);
        for (Format f : values()) {
            if (lower.endsWith("." + f.id)) {
                return f;
            }
        }
        return lower.endsWith(".htm") ? HTML : PDF;
    }
}
