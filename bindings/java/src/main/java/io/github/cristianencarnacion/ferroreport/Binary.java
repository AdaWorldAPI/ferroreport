package io.github.cristianencarnacion.ferroreport;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.nio.file.StandardCopyOption;
import java.security.MessageDigest;
import java.time.Duration;
import java.util.Locale;
import java.util.zip.GZIPInputStream;
import java.util.zip.ZipEntry;
import java.util.zip.ZipInputStream;

/**
 * Finds the engine: the {@code ferroreport} program this library runs.
 *
 * <ol>
 *   <li>the system property {@code ferroreport.bin}, or the environment variable {@code FERROREPORT_BIN};</li>
 *   <li>{@code ferroreport} on the {@code PATH};</li>
 *   <li>the copy this library downloaded before, in the user's cache directory;</li>
 *   <li>a download of the release with this library's version from GitHub, checked against the
 *       release's SHA-256 list. Turn it off with {@code -Dferroreport.download=false} (or
 *       {@code FERROREPORT_DOWNLOAD=0}) where programs may not be fetched at run time, and install the
 *       engine in the image instead.</li>
 * </ol>
 */
final class Binary {
    private static final String RELEASES = "https://github.com/cristianEncarnacion/ferroreport/releases/download/v";
    private static volatile Path found;

    private Binary() {
    }

    static Path locate() {
        Path p = found;
        if (p == null) {
            synchronized (Binary.class) {
                p = found;
                if (p == null) {
                    found = p = find();
                }
            }
        }
        return p;
    }

    private static Path find() {
        String configured = System.getProperty("ferroreport.bin", System.getenv("FERROREPORT_BIN"));
        if (configured != null && !configured.isEmpty()) {
            Path p = Paths.get(configured);
            if (!Files.isRegularFile(p)) {
                throw new FerroreportException("ferroreport.bin / FERROREPORT_BIN points to " + p + ", which is not a file");
            }
            return p;
        }
        String exe = windows() ? "ferroreport.exe" : "ferroreport";
        String path = System.getenv("PATH");
        if (path != null) {
            for (String dir : path.split(java.io.File.pathSeparator)) {
                if (!dir.isEmpty()) {
                    Path p = Paths.get(dir, exe);
                    if (Files.isExecutable(p)) {
                        return p;
                    }
                }
            }
        }
        Path cached = cacheDir().resolve(Ferroreport.VERSION).resolve(exe);
        if (Files.isExecutable(cached)) {
            return cached;
        }
        String download = System.getProperty("ferroreport.download", System.getenv("FERROREPORT_DOWNLOAD"));
        if ("false".equalsIgnoreCase(download) || "0".equals(download)) {
            throw new FerroreportException(
                "The ferroreport engine was not found. Install it (https://github.com/cristianEncarnacion/ferroreport/releases) "
                    + "and put it on the PATH, or point -Dferroreport.bin / FERROREPORT_BIN to it.");
        }
        return download(cached);
    }

    private static boolean windows() {
        return System.getProperty("os.name", "").toLowerCase(Locale.ROOT).contains("win");
    }

    private static Path cacheDir() {
        String xdg = System.getenv("XDG_CACHE_HOME");
        if (xdg != null && !xdg.isEmpty()) {
            return Paths.get(xdg, "ferroreport");
        }
        String local = System.getenv("LOCALAPPDATA");
        if (windows() && local != null && !local.isEmpty()) {
            return Paths.get(local, "ferroreport");
        }
        return Paths.get(System.getProperty("user.home", "."), ".cache", "ferroreport");
    }

    /** The release file for this machine, such as {@code ferroreport-linux-x86_64.tar.gz}. */
    static String asset(String osName, String osArch) {
        String os = osName.toLowerCase(Locale.ROOT);
        String arch = osArch.toLowerCase(Locale.ROOT);
        boolean arm = arch.equals("aarch64") || arch.equals("arm64");
        boolean x64 = arch.equals("amd64") || arch.equals("x86_64");
        if (!arm && !x64) {
            throw new FerroreportException("There is no ferroreport build for the processor `" + osArch + "`: build it from source and set FERROREPORT_BIN.");
        }
        String cpu = arm ? "aarch64" : "x86_64";
        if (os.contains("win")) {
            if (arm) {
                throw new FerroreportException("There is no ferroreport build for Windows on ARM yet: build it from source and set FERROREPORT_BIN.");
            }
            return "ferroreport-windows-x86_64.zip";
        }
        if (os.contains("mac") || os.contains("darwin")) {
            return "ferroreport-macos-" + cpu + ".tar.gz";
        }
        if (os.contains("linux")) {
            return "ferroreport-linux-" + cpu + ".tar.gz";
        }
        throw new FerroreportException("There is no ferroreport build for `" + osName + "`: build it from source and set FERROREPORT_BIN.");
    }

    private static Path download(Path target) {
        String asset = asset(System.getProperty("os.name", ""), System.getProperty("os.arch", ""));
        String base = RELEASES + Ferroreport.VERSION + "/";
        try {
            HttpClient http = HttpClient.newBuilder().followRedirects(HttpClient.Redirect.NORMAL).connectTimeout(Duration.ofSeconds(20)).build();
            byte[] archive = get(http, base + asset);
            String sums = new String(get(http, base + "SHA256SUMS"), StandardCharsets.UTF_8);
            String digest = hex(MessageDigest.getInstance("SHA-256").digest(archive));
            if (!listed(sums, asset, digest)) {
                throw new FerroreportException("The download of " + asset + " does not match the release's SHA256SUMS; nothing was installed.");
            }
            byte[] program = asset.endsWith(".zip") ? fromZip(archive) : fromTarGz(archive);
            Files.createDirectories(target.getParent());
            Path part = Files.createTempFile(target.getParent(), "ferroreport", ".part");
            Files.write(part, program);
            part.toFile().setExecutable(true, false);
            Files.move(part, target, StandardCopyOption.REPLACE_EXISTING);
            return target;
        } catch (IOException | InterruptedException | java.security.GeneralSecurityException e) {
            if (e instanceof InterruptedException) {
                Thread.currentThread().interrupt();
            }
            throw new FerroreportException(
                "Could not download the ferroreport engine " + Ferroreport.VERSION + " (" + e + "). Install it and set FERROREPORT_BIN, "
                    + "or put `ferroreport` on the PATH.", e);
        }
    }

    private static byte[] get(HttpClient http, String url) throws IOException, InterruptedException {
        HttpResponse<byte[]> r = http.send(HttpRequest.newBuilder(URI.create(url)).timeout(Duration.ofMinutes(2)).build(), HttpResponse.BodyHandlers.ofByteArray());
        if (r.statusCode() != 200) {
            throw new IOException(url + " answered " + r.statusCode());
        }
        return r.body();
    }

    /** Whether {@code sums} (lines of "digest  name") lists {@code name} with {@code digest}. */
    static boolean listed(String sums, String name, String digest) {
        for (String line : sums.split("\\R")) {
            String[] parts = line.trim().split("\\s+", 2);
            if (parts.length == 2 && parts[1].replaceFirst("^\\*", "").equals(name)) {
                return parts[0].equalsIgnoreCase(digest);
            }
        }
        return false;
    }

    static String hex(byte[] bytes) {
        StringBuilder s = new StringBuilder();
        for (byte b : bytes) {
            s.append(String.format("%02x", b));
        }
        return s.toString();
    }

    private static boolean isProgram(String entry) {
        String name = entry.substring(entry.lastIndexOf('/') + 1);
        return name.equals("ferroreport") || name.equals("ferroreport.exe");
    }

    private static byte[] fromZip(byte[] archive) throws IOException {
        try (ZipInputStream zip = new ZipInputStream(new java.io.ByteArrayInputStream(archive))) {
            for (ZipEntry e = zip.getNextEntry(); e != null; e = zip.getNextEntry()) {
                if (!e.isDirectory() && isProgram(e.getName())) {
                    return zip.readAllBytes();
                }
            }
        }
        throw new IOException("the archive does not hold the ferroreport program");
    }

    /** Reads the program out of a .tar.gz (the tar format is 512-byte headers followed by the file, padded to 512). */
    static byte[] fromTarGz(byte[] archive) throws IOException {
        try (InputStream in = new GZIPInputStream(new java.io.ByteArrayInputStream(archive))) {
            byte[] header = new byte[512];
            while (in.readNBytes(header, 0, 512) == 512) {
                String name = cString(header, 0, 100);
                if (name.isEmpty()) {
                    break;
                }
                String prefix = cString(header, 345, 155);
                String size = cString(header, 124, 12).trim();
                long length = size.isEmpty() ? 0 : Long.parseLong(size, 8);
                boolean file = header[156] == '0' || header[156] == 0;
                long padded = (length + 511) / 512 * 512;
                if (file && isProgram(prefix.isEmpty() ? name : prefix + "/" + name)) {
                    ByteArrayOutputStream out = new ByteArrayOutputStream((int) length);
                    out.write(in.readNBytes((int) length));
                    return out.toByteArray();
                }
                for (long left = padded; left > 0; ) {
                    long skipped = in.skip(left);
                    if (skipped <= 0) {
                        if (in.read() < 0) {
                            break;
                        }
                        skipped = 1;
                    }
                    left -= skipped;
                }
            }
        }
        throw new IOException("the archive does not hold the ferroreport program");
    }

    private static String cString(byte[] b, int from, int max) {
        int end = from;
        while (end < from + max && b[end] != 0) {
            end++;
        }
        return new String(b, from, end - from, StandardCharsets.UTF_8);
    }
}
