import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import net.sf.jasperreports.engine.*;
import net.sf.jasperreports.engine.data.JRMapCollectionDataSource;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.util.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.AtomicBoolean;

/**
 * JasperReports serving many requests at once: one JVM, the template compiled once, the JIT
 * warmed up, and a number of threads that each fill the report and export the PDF to memory,
 * over and over, for a number of seconds.
 *
 * Usage: Concurrent <report.jrxml> <data.json|-> <params.json|-> <threads> <seconds>
 * Prints {"threads": n, "reports": n, "per_second": x, "p50_ms": x, "p95_ms": x, "p99_ms": x}.
 */
public class Concurrent {
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
        int threads = Integer.parseInt(args[3]);
        double seconds = Double.parseDouble(args[4]);
        Callable<Integer> render = () -> {
            JRDataSource ds = args[1].equals("-") ? new JREmptyDataSource(0) : new JRMapCollectionDataSource(rows);
            JasperPrint print = JasperFillManager.fillReport(report, new HashMap<>(base), ds);
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            JasperExportManager.exportReportToPdfStream(print, out);
            return out.size();
        };
        for (int i = 0; i < 300; i++) render.call();

        AtomicBoolean stop = new AtomicBoolean();
        ExecutorService pool = Executors.newFixedThreadPool(threads);
        List<Future<List<Double>>> results = new ArrayList<>();
        long start = System.nanoTime();
        for (int t = 0; t < threads; t++) {
            results.add(pool.submit(() -> {
                List<Double> ms = new ArrayList<>();
                while (!stop.get()) {
                    long at = System.nanoTime();
                    render.call();
                    ms.add((System.nanoTime() - at) / 1e6);
                }
                return ms;
            }));
        }
        Thread.sleep((long) (seconds * 1000));
        stop.set(true);
        List<Double> all = new ArrayList<>();
        try {
            for (Future<List<Double>> f : results) all.addAll(f.get());
        } catch (ExecutionException e) {
            // A render failed (out of memory, most likely): say so and stop, instead of
            // leaving the other threads waiting.
            System.err.println("a render failed with " + threads + " threads: " + e.getCause());
            System.exit(1);
        }
        double took = (System.nanoTime() - start) / 1e9;
        pool.shutdown();
        Collections.sort(all);
        System.out.printf(Locale.US, "{\"threads\": %d, \"reports\": %d, \"per_second\": %.1f, \"p50_ms\": %.2f, \"p95_ms\": %.2f, \"p99_ms\": %.2f}%n",
            threads, all.size(), all.size() / took, all.get(all.size() / 2), all.get((int) (all.size() * 0.95)), all.get((int) (all.size() * 0.99)));
    }
}
