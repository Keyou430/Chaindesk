package dev.chaindesk.trafficprobe;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.net.DatagramPacket;
import java.net.DatagramSocket;
import java.net.InetAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.KeyStore;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.FutureTask;
import java.util.concurrent.TimeUnit;
import javax.net.ssl.KeyManagerFactory;
import javax.net.ssl.SSLContext;
import javax.net.ssl.SSLServerSocket;

public final class TrafficClientChecks {
    public static void main(String[] args) throws Exception {
        checkLiteralInputs();
        checkCompleteBodyWithoutEof();
        checkMalformedResponses();
        checkUdpNonce();
        checkIpv6Connection();
        checkTls(Path.of(args[0]), true);
        checkTls(Path.of(args[1]), false);
        System.out.println("TrafficClient checks passed");
    }

    private static void checkLiteralInputs() throws Exception {
        check(TrafficClient.literalAddress("127.0.0.1").getHostAddress().equals("127.0.0.1"), "IPv4 literal rejected");
        check(TrafficClient.literalAddress("::1").getAddress().length == 16, "IPv6 literal rejected");
        for (String host : new String[]{"example.com", "https://127.0.0.1", "127.1", "0127.0.0.1", "256.0.0.1", "[::1]", "fe80::1%1", ""}) {
            try {
                TrafficClient.literalAddress(host);
                throw new AssertionError("nonliteral or ambiguous host accepted");
            } catch (IllegalArgumentException expected) {
                // Input validation must finish before any DNS or socket operation.
            }
        }
        for (int port : new int[]{0, 65536, -1}) {
            try {
                TrafficClient.run("udp", "127.0.0.1", port, null);
                throw new AssertionError("invalid port accepted");
            } catch (IllegalArgumentException expected) {
                // A rejected port must not reach DatagramSocket.send.
            }
        }
    }

    private static void checkCompleteBodyWithoutEof() throws Exception {
        CountDownLatch release = new CountDownLatch(1);
        try (ServerSocket listener = loopbackListener()) {
            FutureTask<Void> server = new FutureTask<>(() -> {
                try (Socket socket = listener.accept()) {
                    socket.getOutputStream().write(response("a0-chain-ok"));
                    socket.getOutputStream().flush();
                    check(release.await(3, TimeUnit.SECONDS), "client waited for EOF after complete body");
                }
                return null;
            });
            start(server);
            FutureTask<Void> client = new FutureTask<>(() -> {
                try (Socket socket = new Socket("127.0.0.1", listener.getLocalPort())) {
                    TrafficClient.verifyResponse(socket);
                }
                return null;
            });
            start(client);
            try {
                client.get(2, TimeUnit.SECONDS);
            } finally {
                release.countDown();
            }
            server.get(2, TimeUnit.SECONDS);
        }
    }

    private static void checkMalformedResponses() throws Exception {
        for (String response : new String[]{
                "HTTP/1.1 302 Found\r\nContent-Length: 0\r\n\r\n",
                "HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nbad",
                "HTTP/1.1 200 OK\r\nContent-Length: 11\r\nContent-Length: 11\r\n\r\na0-chain-ok",
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
                "HTTP/1.1 200 OK\r\nContent-Length: 1000000\r\n\r\n",
                "HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\na0",
                "HTTP/1.1 200 OK\r\nX-Long: " + "x".repeat(9000) + "\r\nContent-Length: 10\r\n\r\na0-chain-ok"}) {
            try (ServerSocket listener = loopbackListener()) {
                FutureTask<Void> server = new FutureTask<>(() -> {
                    try (Socket socket = listener.accept()) {
                        socket.getOutputStream().write(response.getBytes(StandardCharsets.US_ASCII));
                    }
                    return null;
                });
                start(server);
                try (Socket socket = new Socket("127.0.0.1", listener.getLocalPort())) {
                    try {
                        TrafficClient.verifyResponse(socket);
                        throw new AssertionError("malformed controlled HTTP response accepted");
                    } catch (java.io.IOException expected) {
                        // Invalid fixture content cannot become a successful chain observation.
                    }
                }
                server.get(2, TimeUnit.SECONDS);
            }
        }
    }

    private static void checkUdpNonce() throws Exception {
        try (DatagramSocket receiver = new DatagramSocket(0, InetAddress.getByName("127.0.0.1"))) {
            receiver.setSoTimeout(2000);
            TrafficClient.Result result = TrafficClient.run("udp", "127.0.0.1", receiver.getLocalPort(), null);
            check(result.success && result.sent && !result.nonce.isEmpty(), "UDP send not reported");
            DatagramPacket packet = new DatagramPacket(new byte[256], 256);
            receiver.receive(packet);
            String received = new String(packet.getData(), 0, packet.getLength(), StandardCharsets.US_ASCII);
            check(received.equals("chaindesk-udp:" + result.nonce), "UDP nonce cannot be correlated");
        }
    }

    private static void checkIpv6Connection() throws Exception {
        try (ServerSocket receiver = new ServerSocket(0, 1, InetAddress.getByName("::1"))) {
            TrafficClient.Result result = TrafficClient.run("ipv6", "::1", receiver.getLocalPort(), null);
            check(result.success && result.connected, "reachable IPv6 literal did not connect");
            receiver.accept().close();
        }
        int closedPort;
        try (ServerSocket receiver = new ServerSocket(0, 1, InetAddress.getByName("::1"))) {
            closedPort = receiver.getLocalPort();
        }
        TrafficClient.Result result = TrafficClient.run("ipv6", "::1", closedPort, null);
        check(!result.success && !result.connected && !result.exception.isEmpty(), "IPv6 connect failure was concealed");
    }

    private static void checkTls(Path keyStorePath, boolean validHost) throws Exception {
        KeyStore keyStore = KeyStore.getInstance("PKCS12");
        try (InputStream input = Files.newInputStream(keyStorePath)) {
            keyStore.load(input, "traffic-fixture".toCharArray());
        }
        KeyManagerFactory keys = KeyManagerFactory.getInstance(KeyManagerFactory.getDefaultAlgorithm());
        keys.init(keyStore, "traffic-fixture".toCharArray());
        SSLContext context = SSLContext.getInstance("TLS");
        context.init(keys.getKeyManagers(), null, null);
        byte[] certificate = keyStore.getCertificate("fixture").getEncoded();
        for (byte[] trusted : new byte[][]{certificate, null}) {
            try (SSLServerSocket listener = (SSLServerSocket) context.getServerSocketFactory().createServerSocket(0, 1, InetAddress.getByName("127.0.0.1"))) {
                FutureTask<Void> server = new FutureTask<>(() -> {
                    try (Socket socket = listener.accept()) {
                        socket.setSoTimeout(2000);
                        String request = readHeaders(socket.getInputStream());
                        check(request.matches("(?s)GET /a0\\?nonce=[0-9a-f]{32} HTTP/1.1\\r\\n.*"), "invalid probe request");
                        socket.getOutputStream().write(response("a0-chain-ok"));
                    } catch (java.io.IOException expected) {
                        if (trusted != null && validHost) {
                            throw expected;
                        }
                    }
                    return null;
                });
                start(server);
                TrafficClient.Result result = TrafficClient.run("tcp", "127.0.0.1", listener.getLocalPort(), trusted);
                check(result.success == (trusted != null && validHost), "TLS trust or hostname validation failed");
                check(result.connected, "TLS failure was misreported as a TCP block");
                server.get(3, TimeUnit.SECONDS);
            }
        }
    }

    private static byte[] response(String body) {
        return ("HTTP/1.1 200 OK\r\nContent-Length: " + body.length() + "\r\nConnection: close\r\n\r\n" + body).getBytes(StandardCharsets.US_ASCII);
    }

    private static String readHeaders(InputStream input) throws Exception {
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        while (bytes.size() < 8192) {
            int next = input.read();
            if (next < 0) {
                break;
            }
            bytes.write(next);
            if (bytes.toString(StandardCharsets.US_ASCII).endsWith("\r\n\r\n")) {
                return bytes.toString(StandardCharsets.US_ASCII);
            }
        }
        throw new AssertionError("missing controlled request headers");
    }

    private static ServerSocket loopbackListener() throws Exception {
        return new ServerSocket(0, 1, InetAddress.getByName("127.0.0.1"));
    }

    private static void start(FutureTask<Void> task) {
        Thread worker = new Thread(task);
        worker.setDaemon(true);
        worker.start();
    }

    private static void check(boolean condition, String message) {
        if (!condition) {
            throw new AssertionError(message);
        }
    }
}
