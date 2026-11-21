import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import net.sf.jasperreports.engine.*;
import net.sf.jasperreports.engine.data.JRMapCollectionDataSource;
import java.io.File;
import java.math.BigDecimal;
import java.text.SimpleDateFormat;
import java.util.*;

/**
 * Reference renderer: compiles a JRXML with JasperReports, fills it with rows from a JSON
 * array (field values converted to each field's declared class) and exports a PDF.
 * Usage: Oracle <report.jrxml> <data.json|-> <out.pdf> [params.json]
 */
public class Oracle {
    public static void main(String[] args) throws Exception {
        ObjectMapper mapper = new ObjectMapper();
        long t0 = System.nanoTime();
        // Subreports: compile every other .jrxml next to the main template into a .jasper file,
        // since JasperReports loads subreports in compiled form.
        File main = new File(args[0]).getAbsoluteFile();
        File[] siblings = main.getParentFile().listFiles((d, n) -> n.endsWith(".jrxml"));
        if (siblings != null) for (File f : siblings) {
            if (f.equals(main)) continue;
            String out = f.getPath().substring(0, f.getPath().length() - 6) + ".jasper";
            JasperCompileManager.compileReportToFile(f.getPath(), out);
        }
        JasperReport report = JasperCompileManager.compileReport(args[0]);
        long t1 = System.nanoTime();
        Map<String, Class<?>> types = new HashMap<>();
        if (report.getFields() != null) for (JRField f : report.getFields()) types.put(f.getName(), f.getValueClass());
        List<Map<String, ?>> rows = new ArrayList<>();
        if (!args[1].equals("-")) {
            for (JsonNode row : mapper.readTree(new File(args[1]))) {
                Map<String, Object> m = new HashMap<>();
                row.fields().forEachRemaining(e -> m.put(e.getKey(), convert(e.getValue(), types.get(e.getKey()))));
                rows.add(m);
            }
        }
        Map<String, Object> params = new HashMap<>();
        params.put(JRParameter.REPORT_LOCALE, Locale.US);
        params.put(JRParameter.REPORT_TIME_ZONE, TimeZone.getTimeZone("UTC"));
        if (args.length > 3) {
            Map<String, Class<?>> ptypes = new HashMap<>();
            for (JRParameter p : report.getParameters()) ptypes.put(p.getName(), p.getValueClass());
            mapper.readTree(new File(args[3])).fields().forEachRemaining(e -> params.put(e.getKey(), convert(e.getValue(), ptypes.get(e.getKey()))));
        }
        JRDataSource ds = args[1].equals("-") ? new JREmptyDataSource(0) : new JRMapCollectionDataSource(rows);
        JasperPrint print = JasperFillManager.fillReport(report, params, ds);
        long t2 = System.nanoTime();
        JasperExportManager.exportReportToPdfFile(print, args[2]);
        long t3 = System.nanoTime();
        System.err.printf("compile_ms=%.1f fill_ms=%.1f export_ms=%.1f pages=%d%n",
            (t1 - t0) / 1e6, (t2 - t1) / 1e6, (t3 - t2) / 1e6, print.getPages().size());
    }

    static Object convert(JsonNode v, Class<?> type) {
        if (v == null || v.isNull()) return null;
        if (v.isArray() && (type == null || type == Object.class || java.util.Collection.class.isAssignableFrom(type))) {
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
        } catch (Exception e) { throw new RuntimeException("cannot convert " + v + " to " + type, e); }
        return v.asText();
    }
}
