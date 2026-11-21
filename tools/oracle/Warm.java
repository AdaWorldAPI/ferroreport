import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import net.sf.jasperreports.engine.*;
import net.sf.jasperreports.engine.data.JRMapCollectionDataSource;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.util.*;

/**
 * JasperReports at its best: one JVM that stays up, the template compiled once, the JIT warmed
 * up, the rows already converted to Java objects. Each timed run fills the report and exports
 * the PDF to memory, which is what a long-running service does per request.
 *
 * Usage: Warm <report.jrxml> <data.json|-> <params.json|-> <warm-up runs> <timed runs>
 * Prints {"runs": n, "median_ms": x, "min_ms": y, "bytes": z}.
 */
public class Warm {
    public static void main(String[] args) throws Exception {
        ObjectMapper mapper = new ObjectMapper();
        JasperReport report = JasperCompileManager.compileReport(args[0]);
        Map<String, Class<?>> types = new HashMap<>();
        if (report.getFields() != null) for (JRField f : report.getFields()) types.put(f.getName(), f.getValueClass());
        List<Map<String, ?>> rows = new ArrayList<>();
        if (!args[1].equals("-")) {
            for (JsonNode row : mapper.readTree(new File(args[1]))) {
                Map<String, Object> m = new HashMap<>();
                row.fields().forEachRemaining(e -> m.put(e.getKey(), Oracle.convert(e.getValue(), types.get(e.getKey()))));
                rows.add(m);
            }
        }
        Map<String, Object> base = new HashMap<>();
        base.put(JRParameter.REPORT_LOCALE, Locale.US);
        base.put(JRParameter.REPORT_TIME_ZONE, TimeZone.getTimeZone("UTC"));
        if (!args[2].equals("-")) {
            Map<String, Class<?>> ptypes = new HashMap<>();
            for (JRParameter p : report.getParameters()) ptypes.put(p.getName(), p.getValueClass());
            mapper.readTree(new File(args[2])).fields()
                .forEachRemaining(e -> base.put(e.getKey(), Oracle.convert(e.getValue(), ptypes.get(e.getKey()))));
        }
        int warm = Integer.parseInt(args[3]), runs = Integer.parseInt(args[4]);
        double[] ms = new double[runs];
        int bytes = 0;
        for (int i = 0; i < warm + runs; i++) {
            long t = System.nanoTime();
            JRDataSource ds = args[1].equals("-") ? new JREmptyDataSource(0) : new JRMapCollectionDataSource(rows);
            JasperPrint print = JasperFillManager.fillReport(report, new HashMap<>(base), ds);
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            JasperExportManager.exportReportToPdfStream(print, out);
            if (i >= warm) ms[i - warm] = (System.nanoTime() - t) / 1e6;
            bytes = out.size();
        }
        Arrays.sort(ms);
        System.out.printf(Locale.US, "{\"runs\": %d, \"median_ms\": %.3f, \"min_ms\": %.3f, \"bytes\": %d}%n", runs, ms[runs / 2], ms[0], bytes);
    }
}
