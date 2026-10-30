package io.sryza;

import java.io.IOException;
import java.io.InputStream;
import java.nio.file.AtomicMoveNotSupportedException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.LinkOption;
import java.nio.file.StandardCopyOption;
import java.nio.file.attribute.PosixFilePermission;
import java.security.MessageDigest;
import java.util.HexFormat;
import java.util.Locale;
import java.util.Set;
import java.util.concurrent.TimeUnit;

/** Pterodactyl-compatible Java entry point for the Rust server engine. */
public final class Bootstrap {
    private Bootstrap() {}

    public static void main(String[] args) throws Exception {
        String platform = platform();
        String file = "sryzamc-" + platform + (platform.startsWith("windows-") ? ".exe" : "");
        String resource = "/natives/" + file;
        Path root = serverRoot(args);
        Path libraries = root.resolve("libraries");
        Files.createDirectories(libraries);
        if (Files.isSymbolicLink(libraries)) throw new SecurityException("Refusing symlink libraries directory");
        Path nativeDir = root.resolve("libraries/native");
        Files.createDirectories(nativeDir);
        if (Files.isSymbolicLink(nativeDir)) throw new SecurityException("Refusing symlink native directory");
        Path executable = nativeDir.resolve(file).normalize();
        if (!executable.getParent().equals(nativeDir)) {
            throw new IOException("Invalid native executable path");
        }
        extractVerified(resource, resource + ".sha256", executable);

        if (args.length == 0 && !Files.exists(root.resolve("sryza.toml"))) {
            int initStatus = runChild(executable, root, new String[] {"init"}, false);
            if (initStatus != 0) System.exit(initStatus);
        }
        System.exit(runChild(executable, root, args, true));
    }

    private static int runChild(Path executable, Path root, String[] args, boolean manageShutdown)
            throws Exception {
        ProcessBuilder builder = new ProcessBuilder(command(executable, root, args));
        builder.directory(root.toFile());
        builder.inheritIO();
        Process child = builder.start();
        if (manageShutdown) {
            Runtime.getRuntime().addShutdownHook(new Thread(() -> {
                stopChild(child);
                if (!child.isAlive()) Runtime.getRuntime().halt(child.exitValue());
            }, "sryzamc-shutdown"));
        }
        return child.waitFor();
    }

    private static String platform() {
        String os = System.getProperty("os.name", "").toLowerCase(Locale.ROOT);
        String arch = System.getProperty("os.arch", "").toLowerCase(Locale.ROOT);
        String cpu = switch (arch) {
            case "amd64", "x86_64" -> "x86_64";
            case "aarch64", "arm64" -> "aarch64";
            default -> throw new IllegalStateException("Unsupported CPU architecture: " + arch);
        };
        if (os.contains("linux")) return "linux-" + cpu;
        if (os.contains("windows") && cpu.equals("x86_64")) return "windows-x86_64";
        throw new IllegalStateException("Unsupported operating system: " + os + " / " + cpu);
    }

    private static void extractVerified(String binaryResource, String hashResource, Path destination)
            throws Exception {
        byte[] binary;
        try (InputStream input = Bootstrap.class.getResourceAsStream(binaryResource)) {
            if (input == null) throw new IOException("server.jar does not contain " + binaryResource);
            binary = input.readNBytes(256 * 1024 * 1024 + 1);
            if (binary.length > 256 * 1024 * 1024) throw new IOException("Bundled native exceeds the 256 MiB extraction limit");
        }
        String expected;
        try (InputStream input = Bootstrap.class.getResourceAsStream(hashResource)) {
            if (input == null) throw new IOException("server.jar does not contain " + hashResource);
            expected = new String(input.readAllBytes(), java.nio.charset.StandardCharsets.US_ASCII)
                    .trim().toLowerCase(Locale.ROOT);
        }
        String actual = HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(binary));
        if (expected.length() != 64 || !MessageDigest.isEqual(
                expected.getBytes(java.nio.charset.StandardCharsets.US_ASCII),
                actual.getBytes(java.nio.charset.StandardCharsets.US_ASCII))) {
            throw new SecurityException("Bundled native SHA-256 mismatch for " + binaryResource);
        }
        if (Files.isSymbolicLink(destination)) Files.delete(destination);
        if (Files.isRegularFile(destination, LinkOption.NOFOLLOW_LINKS)
                && MessageDigest.isEqual(hash(destination), actual.getBytes(java.nio.charset.StandardCharsets.US_ASCII))) {
            return;
        }
        Path temporary = Files.createTempFile(destination.getParent(), "sryzamc-", ".tmp");
        try {
            Files.write(temporary, binary);
            try {
                Files.setPosixFilePermissions(temporary, Set.of(
                        PosixFilePermission.OWNER_READ, PosixFilePermission.OWNER_WRITE,
                        PosixFilePermission.OWNER_EXECUTE, PosixFilePermission.GROUP_READ,
                        PosixFilePermission.GROUP_EXECUTE));
            } catch (UnsupportedOperationException ignored) {
                temporary.toFile().setExecutable(true, true);
            }
            try {
                Files.move(temporary, destination, StandardCopyOption.ATOMIC_MOVE,
                        StandardCopyOption.REPLACE_EXISTING);
            } catch (AtomicMoveNotSupportedException ignored) {
                Files.move(temporary, destination, StandardCopyOption.REPLACE_EXISTING);
            }
        } finally {
            Files.deleteIfExists(temporary);
        }
    }

    private static byte[] hash(Path file) throws Exception {
        return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(Files.readAllBytes(file)))
                .getBytes(java.nio.charset.StandardCharsets.US_ASCII);
    }

    private static java.util.List<String> command(Path executable, Path root, String[] args) {
        var command = new java.util.ArrayList<String>();
        command.add(executable.toString());
        if (args.length == 0) {
            command.add("--dir");
            command.add(root.toString());
            command.add("run");
        } else {
            command.addAll(java.util.List.of(args));
        }
        return command;
    }

    private static Path serverRoot(String[] args) {
        Path root = Path.of(System.getProperty("user.dir"));
        for (int i = 0; i < args.length; i++) {
            if (args[i].equals("--dir")) {
                if (i + 1 >= args.length) throw new IllegalArgumentException("--dir requires a path");
                root = Path.of(args[++i]);
            } else if (args[i].startsWith("--dir=")) {
                root = Path.of(args[i].substring("--dir=".length()));
            }
        }
        return root.toAbsolutePath().normalize();
    }

    private static void stopChild(Process child) {
        if (!child.isAlive()) return;
        child.destroy();
        try {
            if (!child.waitFor(10, TimeUnit.SECONDS)) child.destroyForcibly();
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            child.destroyForcibly();
        }
    }
}
