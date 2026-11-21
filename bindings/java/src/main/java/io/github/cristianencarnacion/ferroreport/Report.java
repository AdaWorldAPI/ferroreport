package io.github.cristianencarnacion.ferroreport;

import java.io.BufferedWriter;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.io.OutputStreamWriter;
import java.io.Writer;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.sql.ResultSet;
import java.sql.ResultSetMetaData;
import java.sql.SQLException;
import java.time.Duration;
import java.time.LocalDateTime;
import java.util.ArrayList;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.NoSuchElementException;
import java.util.Objects;
import java.util.concurrent.atomic.AtomicReference;

/**
 * One report about to be rendered: its template, rows, parameters and options. Build it from a
 * {@link Ferroreport}, then call {@link #render()} or one of the {@code renderTo} methods. A
 * {@code Report} is used once and by one thread.
 */
public final class Report {
    private final Ferroreport engine;
    private final Path template;
    private final String source;
    private final String name;
    private Iterable<?> rows;
    private final Map<String, Object> params = new LinkedHashMap<>();
    private final List<Path> resources = new ArrayList<>();
    private String locale = "en_US";
    private Format format;
    private Integer maxPages;
    private Duration timeout;
    private LocalDateTime now;
    private boolean csvSafe;
    private boolean xlsxNumbers;

    Report(Ferroreport engine, Path template, String source, String name) {
        this.engine = engine;
        this.template = template;
        this.source = source;
        this.name = name;
    }

    /**
     * The rows of the report: Maps from field name to value, or anything else the template's fields
     * can be read from as JSON (nested Maps and Lists feed subreports and tables). They are read
     * one at a time while the report renders, so a lazy Iterable (a cursor, a stream's iterator)
     * is never held whole in memory.
     */
    public Report rows(Iterable<?> rows) {
        this.rows = Objects.requireNonNull(rows, "rows");
        return this;
    }

    /**
     * The rows of a query, read from the {@code ResultSet} as the report needs them: column labels
     * are the field names. The ResultSet stays yours to close. For a result larger than memory, ask
     * the driver to stream it (a fetch size on PostgreSQL, {@code Integer.MIN_VALUE} on MySQL).
     */
    public Report rows(ResultSet results) {
        Objects.requireNonNull(results, "results");
        this.rows = (Iterable<Map<String, Object>>) () -> new ResultSetRows(results);
        return this;
    }

    /** Sets a report parameter ({@code $P{name}} in the template). */
    public Report param(String name, Object value) {
        params.put(Objects.requireNonNull(name, "name"), value);
        return this;
    }

    /** Sets several report parameters. */
    public Report params(Map<String, ?> values) {
        params.putAll(values);
        return this;
    }

    /** Number and date formatting: {@code en_US} (the default), {@code es_DO}, {@code es_MX}, {@code es_ES}... */
    public Report locale(String tag) {
        this.locale = Objects.requireNonNull(tag, "tag");
        return this;
    }

    /** The output format. Without it, {@code renderTo(Path)} goes by the file's extension and everything else gives PDF. */
    public Report format(Format format) {
        this.format = Objects.requireNonNull(format, "format");
        return this;
    }

    /** An extra directory for images and subreports (local engine). The template's own directory is always searched. */
    public Report resources(Path directory) {
        resources.add(Objects.requireNonNull(directory, "directory"));
        return this;
    }

    /** Stops with a {@link FerroreportException} if the report goes over this many pages (local engine). */
    public Report maxPages(int pages) {
        this.maxPages = pages;
        return this;
    }

    /** Stops with a {@link FerroreportException} if rendering takes longer than this (local engine). */
    public Report timeout(Duration longest) {
        this.timeout = Objects.requireNonNull(longest, "longest");
        return this;
    }

    /** Fixes the value of {@code new Date()} in the template's expressions, for reproducible output. */
    public Report now(LocalDateTime now) {
        this.now = Objects.requireNonNull(now, "now");
        return this;
    }

    /** In CSV, puts an apostrophe before values a spreadsheet would run as a formula. Use it when the data comes from other people. */
    public Report csvSafe(boolean on) {
        this.csvSafe = on;
        return this;
    }

    /** In Excel, writes numbers as numbers instead of as the formatted text. */
    public Report xlsxNumbers(boolean on) {
        this.xlsxNumbers = on;
        return this;
    }

    /** Renders the report and returns the document's bytes. */
    public byte[] render() {
        ByteArrayOutputStream out = new ByteArrayOutputStream(1 << 16);
        run(out, format != null ? format : Format.PDF);
        return out.toByteArray();
    }

    /**
     * Renders the report to a file, page by page. The format follows the extension
     * ({@code .pdf}, {@code .xlsx}, {@code .csv}, {@code .html}) unless {@link #format} was set. If
     * rendering fails, no file is left behind.
     */
    public void renderTo(Path file) {
        Format f = format != null ? format : Format.of(file.getFileName().toString());
        Path dir = file.toAbsolutePath().getParent();
        Path part = null;
        try {
            part = Files.createTempFile(dir, file.getFileName().toString(), ".part");
            try (OutputStream out = Files.newOutputStream(part)) {
                run(out, f);
            }
            Files.move(part, file, StandardCopyOption.REPLACE_EXISTING);
            part = null;
        } catch (IOException e) {
            throw new FerroreportException("cannot write " + file + ": " + e.getMessage(), e);
        } finally {
            if (part != null) {
                try {
                    Files.deleteIfExists(part);
                } catch (IOException ignored) {
                    // The report's own error is the one to show.
                }
            }
        }
    }

    /**
     * Renders the report to a stream as it is made (an HTTP response, for example); the stream is
     * not closed. If rendering fails part of the way, what was already written stays written: use
     * {@link #render()} when the document must arrive whole or not at all.
     */
    public void renderTo(OutputStream out) {
        run(Objects.requireNonNull(out, "out"), format != null ? format : Format.PDF);
    }

    private void run(OutputStream out, Format f) {
        if (engine.server != null) {
            remote(out, f);
        } else {
            local(out, f);
        }
    }

    // ---------------------------------------------------------------------------------------
    // The engine as a child process: rows go in through its standard input as they are read,
    // the document comes out through its standard output as it is made.

    private void local(OutputStream out, Format f) {
        Path temp = null;
        Path paramsFile = null;
        try {
            Path jrxml = template;
            if (jrxml == null) {
                temp = Files.createTempFile("ferroreport", ".jrxml");
                Files.write(temp, source.getBytes(StandardCharsets.UTF_8));
                jrxml = temp;
            }
            List<String> cmd = new ArrayList<>(List.of(engine.binary().toString(), "render", jrxml.toString(), "-o", "-", "-f", f.id(), "-q", "-l", locale));
            if (rows != null) {
                cmd.add("-d");
                cmd.add("-");
            }
            if (!params.isEmpty()) {
                paramsFile = Files.createTempFile("ferroreport", ".json");
                Files.write(paramsFile, Json.text(params).getBytes(StandardCharsets.UTF_8));
                cmd.add("-p");
                cmd.add(paramsFile.toString());
            }
            for (Path dir : resources) {
                cmd.add("-r");
                cmd.add(dir.toString());
            }
            if (maxPages != null) {
                cmd.add("--max-pages");
                cmd.add(maxPages.toString());
            }
            if (timeout != null) {
                cmd.add("--timeout");
                cmd.add(String.valueOf(timeout.toMillis() / 1000.0));
            }
            if (now != null) {
                cmd.add("--now");
                cmd.add(now.withNano(0).toString().length() == 16 ? now.withNano(0) + ":00" : now.withNano(0).toString());
            }
            if (csvSafe) {
                cmd.add("--csv-safe");
            }
            if (xlsxNumbers) {
                cmd.add("--xlsx-numbers");
            }
            Process process = new ProcessBuilder(cmd).start();
            AtomicReference<Throwable> rowsFailure = new AtomicReference<>();
            Thread feeder = new Thread(() -> feed(process, rowsFailure), "ferroreport-rows");
            ByteArrayOutputStream errors = new ByteArrayOutputStream();
            Thread listener = new Thread(() -> drain(process.getErrorStream(), errors), "ferroreport-errors");
            feeder.setDaemon(true);
            listener.setDaemon(true);
            feeder.start();
            listener.start();
            try (InputStream document = process.getInputStream()) {
                document.transferTo(out);
            } catch (IOException e) {
                process.destroyForcibly();
                feeder.join(2000);
                // When the rows failed, the engine was stopped on purpose: that is the error to report.
                if (rowsFailure.get() == null) {
                    throw new FerroreportException("cannot write the document: " + e.getMessage(), e);
                }
            }
            int exit = process.waitFor();
            feeder.join();
            listener.join();
            Throwable failed = rowsFailure.get();
            if (failed != null) {
                throw new FerroreportException("cannot read the rows: " + failed.getMessage(), failed);
            }
            if (exit != 0) {
                String message = new String(errors.toByteArray(), StandardCharsets.UTF_8).trim();
                // The engine's last word is a line that starts with "error: " (warnings may come before it).
                int at = message.startsWith("error: ") ? 0 : message.lastIndexOf("\nerror: ") + 1;
                boolean found = message.startsWith("error: ", at);
                throw new FerroreportException(found ? message.substring(at + 7) : message.isEmpty() ? "the engine stopped with status " + exit : message);
            }
        } catch (IOException e) {
            throw new FerroreportException("cannot run the ferroreport engine: " + e.getMessage(), e);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new FerroreportException("interrupted while rendering", e);
        } finally {
            delete(temp);
            delete(paramsFile);
        }
    }

    /** Writes the rows as JSON, one per line, as the engine asks for them. */
    private void feed(Process process, AtomicReference<Throwable> failure) {
        try (Writer w = new BufferedWriter(new OutputStreamWriter(process.getOutputStream(), StandardCharsets.UTF_8), 1 << 16)) {
            if (rows != null) {
                for (Object row : rows) {
                    Json.write(row, w);
                    w.write('\n');
                }
            }
        } catch (IOException e) {
            // The engine closed its input: it stopped (an error it reports itself) or needed no more rows.
        } catch (RuntimeException e) {
            failure.set(e.getCause() instanceof SQLException ? e.getCause() : e);
            process.destroyForcibly();
        }
    }

    private static void drain(InputStream in, OutputStream out) {
        try (in) {
            in.transferTo(out);
        } catch (IOException ignored) {
            // Nothing more to read.
        }
    }

    private static void delete(Path file) {
        if (file != null) {
            try {
                Files.deleteIfExists(file);
            } catch (IOException ignored) {
                // A leftover temporary file is not worth failing a report for.
            }
        }
    }

    // ---------------------------------------------------------------------------------------
    // The engine as a server.

    private void remote(OutputStream out, Format f) {
        StringBuilder body = new StringBuilder(1 << 12);
        try {
            body.append('{');
            if (name == null) {
                body.append("\"template\":");
                Json.string(source != null ? source : new String(Files.readAllBytes(template), StandardCharsets.UTF_8), body);
                body.append(',');
            }
            body.append("\"format\":\"").append(f.id()).append("\",\"locale\":");
            Json.string(locale, body);
            if (now != null) {
                body.append(",\"now\":");
                Json.string(now.toString(), body);
            }
            if (csvSafe) {
                body.append(",\"csvSafe\":true");
            }
            if (xlsxNumbers) {
                body.append(",\"xlsxNumbers\":true");
            }
            body.append(",\"params\":");
            Json.write(params, body);
            body.append(",\"data\":");
            Json.write(rows != null ? rows : List.of(), body);
            body.append('}');

            String base = engine.server.toString().replaceAll("/+$", "");
            URI uri = URI.create(base + (name == null ? "/render" : "/render/" + name.replaceAll("^/+", "")));
            HttpRequest.Builder request = HttpRequest.newBuilder(uri)
                .header("Content-Type", "application/json")
                .POST(HttpRequest.BodyPublishers.ofString(body.toString(), StandardCharsets.UTF_8));
            if (engine.token != null) {
                request.header("Authorization", "Bearer " + engine.token);
            }
            HttpResponse<InputStream> response = HTTP.send(request.build(), HttpResponse.BodyHandlers.ofInputStream());
            try (InputStream document = response.body()) {
                if (response.statusCode() != 200) {
                    String text = new String(document.readAllBytes(), StandardCharsets.UTF_8);
                    throw new FerroreportException(errorOf(text, response.statusCode()));
                }
                document.transferTo(out);
            }
        } catch (IOException e) {
            throw new FerroreportException("cannot reach the ferroreport server at " + engine.server + ": " + e.getMessage(), e);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new FerroreportException("interrupted while rendering", e);
        }
    }

    private static final HttpClient HTTP =
        HttpClient.newBuilder().version(HttpClient.Version.HTTP_1_1).connectTimeout(Duration.ofSeconds(10)).build();

    /** The message in a server's {@code {"error": "..."}} answer. */
    static String errorOf(String body, int status) {
        int key = body.indexOf("\"error\"");
        int start = key < 0 ? -1 : body.indexOf('"', body.indexOf(':', key) + 1);
        if (start < 0) {
            return "the server answered " + status + (body.isBlank() ? "" : ": " + body.trim());
        }
        StringBuilder message = new StringBuilder();
        for (int i = start + 1; i < body.length(); i++) {
            char c = body.charAt(i);
            if (c == '"') {
                break;
            }
            if (c == '\\' && i + 1 < body.length()) {
                char next = body.charAt(++i);
                if (next == 'n') {
                    message.append('\n');
                } else if (next == 't') {
                    message.append('\t');
                } else if (next == 'u' && i + 4 < body.length()) {
                    message.append((char) Integer.parseInt(body.substring(i + 1, i + 5), 16));
                    i += 4;
                } else {
                    message.append(next);
                }
            } else {
                message.append(c);
            }
        }
        return message.toString();
    }

    /** The rows of a ResultSet as Maps, read one at a time. */
    private static final class ResultSetRows implements Iterator<Map<String, Object>> {
        private final ResultSet results;
        private final String[] labels;
        private Boolean ready;

        ResultSetRows(ResultSet results) {
            this.results = results;
            try {
                ResultSetMetaData meta = results.getMetaData();
                labels = new String[meta.getColumnCount()];
                for (int i = 0; i < labels.length; i++) {
                    labels[i] = meta.getColumnLabel(i + 1);
                }
            } catch (SQLException e) {
                throw new IllegalStateException(e);
            }
        }

        @Override
        public boolean hasNext() {
            if (ready == null) {
                try {
                    ready = results.next();
                } catch (SQLException e) {
                    throw new IllegalStateException(e);
                }
            }
            return ready;
        }

        @Override
        public Map<String, Object> next() {
            if (!hasNext()) {
                throw new NoSuchElementException();
            }
            ready = null;
            Map<String, Object> row = new LinkedHashMap<>(labels.length * 2);
            try {
                for (int i = 0; i < labels.length; i++) {
                    Object value = results.getObject(i + 1);
                    if (value instanceof java.sql.Clob) {
                        java.sql.Clob clob = (java.sql.Clob) value;
                        value = clob.getSubString(1, (int) Math.min(clob.length(), Integer.MAX_VALUE));
                    } else if (value instanceof java.sql.Blob) {
                        java.sql.Blob blob = (java.sql.Blob) value;
                        value = blob.getBytes(1, (int) Math.min(blob.length(), Integer.MAX_VALUE));
                    }
                    row.put(labels[i], value);
                }
            } catch (SQLException e) {
                throw new IllegalStateException(e);
            }
            return row;
        }
    }
}
