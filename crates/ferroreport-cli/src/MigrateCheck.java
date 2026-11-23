import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import net.sf.jasperreports.engine.*;
import net.sf.jasperreports.engine.data.JRMapCollectionDataSource;
import java.io.*;
import java.math.BigDecimal;
import java.nio.charset.StandardCharsets;
import java.text.SimpleDateFormat;
import java.util.*;

/**
 * Run by `ferroreport migrate --jasper <lib>`: renders templates with the JasperReports jars in
 * that folder, in one JVM, so ferroreport can compare its output with Jasper's.
 *
 * Reads one job per line on standard input, separated by tabs:
 *   id, template, rows (JSON array, or -), parameters (JSON object, or -), output stem,
 *   the template's own folder
 * and writes the output stem with .pdf and .csv. Images, styles and subreports are looked up
 * in the output's folder first (where subreports are compiled) and then in the template's own
 * folder, which is never written to. Answers one line per job on standard output:
 *   id, ok|error, pages, compile milliseconds, fill and PDF export milliseconds, message
 * Rows and parameters are converted to the classes the template declares; the locale is en_US
 * and the time zone UTC, as ferroreport's defaults.
 */
public class MigrateCheck {
    public static void main(String[] args) throws Exception {
        ObjectMapper mapper = new ObjectMapper();
        BufferedReader in = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
        PrintStream out = new PrintStream(new FileOutputStream(FileDescriptor.out), true, "UTF-8");
        String line;
        while ((line = in.readLine()) != null) {
            String[] job = line.split("\t", -1);
            if (job.length < 6) continue;
            String id = job[0];
            try {
                long t0 = System.nanoTime();
                File main = new File(job[1]).getAbsoluteFile();
                File work = new File(job[4]).getAbsoluteFile().getParentFile();
                File source = new File(job[5]).getAbsoluteFile();
                SimpleJasperReportsContext ctx = new SimpleJasperReportsContext();
                ctx.setExtensions(net.sf.jasperreports.repo.RepositoryService.class, Arrays.asList(
                    new net.sf.jasperreports.repo.FileRepositoryService(ctx, work.getPath(), false),
                    new net.sf.jasperreports.repo.FileRepositoryService(ctx, source.getPath(), false)));
                List<net.sf.jasperreports.repo.PersistenceServiceFactory> persistence = new ArrayList<>();
                persistence.add(net.sf.jasperreports.repo.FileRepositoryPersistenceServiceFactory.getInstance());
                persistence.addAll(DefaultJasperReportsContext.getInstance().getExtensions(net.sf.jasperreports.repo.PersistenceServiceFactory.class));
                ctx.setExtensions(net.sf.jasperreports.repo.PersistenceServiceFactory.class, persistence);
                // Subreports load in compiled form: compile the templates next to this one into the work folder.
                File[] siblings = source.listFiles((d, n) -> n.endsWith(".jrxml"));
                if (siblings != null) for (File f : siblings) {
                    if (f.getName().equals(main.getName())) continue;
                    try {
                        JasperCompileManager.getInstance(ctx).compileToFile(f.getPath(),
                            new File(work, f.getName().replaceAll("\\.jrxml$", ".jasper")).getPath());
                    } catch (Throwable ignored) {
                        // Not a subreport of this one, or one Jasper cannot compile: the fill says so if it matters.
                    }
                }
                JasperReport report = JasperCompileManager.getInstance(ctx).compile(main.getPath());
                long t1 = System.nanoTime();
                Map<String, Class<?>> types = new HashMap<>();
                if (report.getFields() != null) for (JRField f : report.getFields()) types.put(f.getName(), f.getValueClass());
                Map<String, Object> params = new HashMap<>();
                params.put(JRParameter.REPORT_LOCALE, Locale.US);
                params.put(JRParameter.REPORT_TIME_ZONE, TimeZone.getTimeZone("UTC"));
                if (!job[3].equals("-")) {
                    Map<String, Class<?>> ptypes = new HashMap<>();
                    for (JRParameter p : report.getParameters()) ptypes.put(p.getName(), p.getValueClass());
                    mapper.readTree(new File(job[3])).fields().forEachRemaining(e -> params.put(e.getKey(), convert(e.getValue(), ptypes.get(e.getKey()))));
                }
                JRDataSource ds;
                if (job[2].equals("-")) {
                    ds = new JREmptyDataSource(0);
                } else {
                    List<Map<String, ?>> rows = new ArrayList<>();
                    for (JsonNode row : mapper.readTree(new File(job[2]))) {
                        Map<String, Object> m = new HashMap<>();
                        row.fields().forEachRemaining(e -> m.put(e.getKey(), convert(e.getValue(), types.get(e.getKey()))));
                        rows.add(m);
                    }
                    ds = new JRMapCollectionDataSource(rows);
                }
                JasperPrint print = JasperFillManager.getInstance(ctx).fill(report, params, ds);
                JasperExportManager.getInstance(ctx).exportToPdfFile(print, job[4] + ".pdf");
                long t2 = System.nanoTime();
                net.sf.jasperreports.engine.export.JRCsvExporter csv = new net.sf.jasperreports.engine.export.JRCsvExporter(ctx);
                csv.setExporterInput(new net.sf.jasperreports.export.SimpleExporterInput(print));
                csv.setExporterOutput(new net.sf.jasperreports.export.SimpleWriterExporterOutput(job[4] + ".csv"));
                csv.exportReport();
                out.printf(Locale.ROOT, "%s\tok\t%d\t%.1f\t%.1f\t%n", id, print.getPages().size(), (t1 - t0) / 1e6, (t2 - t1) / 1e6);
            } catch (Throwable e) {
                Throwable root = e;
                while (root.getCause() != null && root.getCause() != root) root = root.getCause();
                String message = (e == root ? "" : e.getClass().getSimpleName() + ": " + e.getMessage() + " / ")
                    + root.getClass().getSimpleName() + ": " + root.getMessage();
                out.printf("%s\terror\t0\t0\t0\t%s%n", id, message.replaceAll("[\\t\\r\\n]+", " "));
            }
        }
    }

    static Object convert(JsonNode v, Class<?> type) {
        if (v == null || v.isNull()) return null;
        if (v.isArray() && (type == null || type == Object.class || Collection.class.isAssignableFrom(type))) {
            // Nested rows for subreports (JRMapCollectionDataSource): plain Java values.
            List<Map<String, ?>> list = new ArrayList<>();
            for (JsonNode item : v) {
                Map<String, Object> m = new HashMap<>();
                item.fields().forEachRemaining(e -> {
                    JsonNode x = e.getValue();
                    Object o = x.isNull() ? null : x.isInt() ? (Object) x.asInt() : x.isIntegralNumber() ? (Object) x.asLong()
                        : x.isNumber() ? (Object) x.asDouble() : x.isBoolean() ? (Object) x.asBoolean() : x.asText();
                    m.put(e.getKey(), o);
                });
                list.add(m);
            }
            return list;
        }
        if (type == null || type == Object.class) {
            if (v.isTextual()) return v.asText();
            if (v.isIntegralNumber()) return v.asLong();
            if (v.isNumber()) return v.asDouble();
            if (v.isBoolean()) return v.asBoolean();
            return v.toString();
        }
        try {
            if (type == String.class) return v.asText();
            if (type == Integer.class) return v.asInt();
            if (type == Long.class) return v.asLong();
            if (type == Short.class) return (short) v.asInt();
            if (type == Double.class) return v.asDouble();
            if (type == Float.class) return (float) v.asDouble();
            if (type == BigDecimal.class) return new BigDecimal(v.asText());
            if (type == Boolean.class) return v.asBoolean();
            if (Date.class.isAssignableFrom(type)) {
                String s = v.asText();
                SimpleDateFormat f = new SimpleDateFormat(s.length() > 10 ? "yyyy-MM-dd'T'HH:mm:ss" : "yyyy-MM-dd");
                f.setTimeZone(TimeZone.getTimeZone("UTC"));
                Date d = f.parse(s);
                if (type == java.sql.Timestamp.class) return new java.sql.Timestamp(d.getTime());
                if (type == java.sql.Date.class) return new java.sql.Date(d.getTime());
                return d;
            }
        } catch (Exception e) {
            throw new RuntimeException("cannot convert " + v + " to " + type, e);
        }
        return v.asText();
    }
}
