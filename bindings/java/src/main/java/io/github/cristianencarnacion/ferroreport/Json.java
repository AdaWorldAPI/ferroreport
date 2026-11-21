package io.github.cristianencarnacion.ferroreport;

import java.io.IOException;
import java.math.BigDecimal;
import java.math.BigInteger;
import java.time.Instant;
import java.time.ZoneOffset;
import java.time.temporal.TemporalAccessor;
import java.util.Base64;
import java.util.Map;

/** Writes Java values as the JSON the engine reads. No dependencies: a report library should not pick your JSON library. */
final class Json {
    private Json() {
    }

    static String text(Object value) {
        StringBuilder out = new StringBuilder();
        try {
            write(value, out);
        } catch (IOException e) {
            throw new IllegalStateException(e);
        }
        return out.toString();
    }

    static void write(Object v, Appendable out) throws IOException {
        if (v == null) {
            out.append("null");
        } else if (v instanceof CharSequence || v instanceof Character || v instanceof Enum || v instanceof java.util.UUID) {
            string(v.toString(), out);
        } else if (v instanceof Boolean) {
            out.append(v.toString());
        } else if (v instanceof BigDecimal) {
            // As text, so that every digit arrives: 185.00 stays 185.00.
            string(((BigDecimal) v).toPlainString(), out);
        } else if (v instanceof Double || v instanceof Float) {
            double d = ((Number) v).doubleValue();
            out.append(Double.isFinite(d) ? v.toString() : "null");
        } else if (v instanceof BigInteger || v instanceof Number) {
            out.append(v.toString());
        } else if (v instanceof java.sql.Timestamp) {
            string(((java.sql.Timestamp) v).toLocalDateTime().toString(), out);
        } else if (v instanceof java.sql.Date || v instanceof java.sql.Time) {
            string(v.toString(), out);
        } else if (v instanceof java.util.Date) {
            string(((java.util.Date) v).toInstant().atOffset(ZoneOffset.UTC).toLocalDateTime().toString(), out);
        } else if (v instanceof Instant) {
            string(((Instant) v).atOffset(ZoneOffset.UTC).toLocalDateTime().toString(), out);
        } else if (v instanceof TemporalAccessor) {
            // LocalDate, LocalDateTime, OffsetDateTime...: their ISO-8601 text.
            string(v.toString(), out);
        } else if (v instanceof byte[]) {
            string(Base64.getEncoder().encodeToString((byte[]) v), out);
        } else if (v instanceof Map) {
            out.append('{');
            boolean first = true;
            for (Map.Entry<?, ?> e : ((Map<?, ?>) v).entrySet()) {
                if (!first) {
                    out.append(',');
                }
                first = false;
                string(String.valueOf(e.getKey()), out);
                out.append(':');
                write(e.getValue(), out);
            }
            out.append('}');
        } else if (v instanceof Iterable) {
            out.append('[');
            boolean first = true;
            for (Object item : (Iterable<?>) v) {
                if (!first) {
                    out.append(',');
                }
                first = false;
                write(item, out);
            }
            out.append(']');
        } else if (v instanceof Object[]) {
            write(java.util.Arrays.asList((Object[]) v), out);
        } else if (v instanceof java.util.Optional) {
            write(((java.util.Optional<?>) v).orElse(null), out);
        } else {
            string(v.toString(), out);
        }
    }

    static void string(String s, Appendable out) throws IOException {
        out.append('"');
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
                case '"':
                    out.append("\\\"");
                    break;
                case '\\':
                    out.append("\\\\");
                    break;
                case '\n':
                    out.append("\\n");
                    break;
                case '\r':
                    out.append("\\r");
                    break;
                case '\t':
                    out.append("\\t");
                    break;
                default:
                    if (c < 0x20 || Character.isSurrogate(c) && !validPair(s, i)) {
                        out.append(String.format("\\u%04x", (int) c));
                    } else {
                        out.append(c);
                    }
            }
        }
        out.append('"');
    }

    /** Whether the surrogate at {@code i} is half of a well-formed pair. */
    private static boolean validPair(String s, int i) {
        char c = s.charAt(i);
        if (Character.isHighSurrogate(c)) {
            return i + 1 < s.length() && Character.isLowSurrogate(s.charAt(i + 1));
        }
        return i > 0 && Character.isHighSurrogate(s.charAt(i - 1));
    }
}
