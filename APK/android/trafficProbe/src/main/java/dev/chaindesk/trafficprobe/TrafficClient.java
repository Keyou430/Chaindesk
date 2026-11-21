package dev.chaindesk.trafficprobe;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.net.DatagramPacket;
import java.net.DatagramSocket;
import java.net.Inet6Address;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.net.SocketTimeoutException;
import java.nio.charset.StandardCharsets;
import java.security.KeyStore;
import java.security.cert.CertificateFactory;
import java.util.Collections;
import java.util.Locale;
import java.util.UUID;
import java.util.concurrent.TimeUnit;
import javax.net.ssl.SNIHostName;
import javax.net.ssl.SSLContext;
import javax.net.ssl.SSLParameters;
import javax.net.ssl.SSLSocket;
import javax.net.ssl.SSLSocketFactory;
import javax.net.ssl.TrustManagerFactory;

final class TrafficClient {
    private static final int TIMEOUT_MS = 5000;
    private static final String TLS_HOST = "example.com";

    static final class Result {
        boolean success;
        boolean connected;
        boolean sent;
        String nonce = "";
        String exception = "";
    }

    static InetAddress literalAddress(String host) throws IOException {
        if (host == null || host.isEmpty() || host.length() > 45) {
            throw new IllegalArgumentException();
        }
        if (host.indexOf(':') >= 0) {
            if (!host.matches("[0-9a-fA-F:.]+")) {
                throw new IllegalArgumentException();
            }
            InetAddress address;
            try {
                address = InetAddress.getByName(host);
            } catch (java.net.UnknownHostException failure) {
                throw new IllegalArgumentException();
            }
            if (!(address instanceof Inet6Address)) {
                throw new IllegalArgumentException();
            }
            return address;
        }
        String[] octets = host.split("\\.", -1);
        if (octets.length != 4) {
            throw new IllegalArgumentException();
        }
        byte[] address = new byte[4];
        for (int index = 0; index < octets.length; index++) {
            String octet = octets[index];
            if (!octet.matches("0|[1-9][0-9]{0,2}")) {
                throw new IllegalArgumentException();
            }
            int value = Integer.parseInt(octet);
            if (value > 255) {
                throw new IllegalArgumentException();
            }
            address[index] = (byte) value;
        }
        return InetAddress.getByAddress(address);
    }

    static Result run(String mode, String host, int port, byte[] certificate) throws Exception {
        if (!("tcp".equals(mode) || "udp".equals(mode) || "ipv6".equals(mode)) || port < 1 || port > 65535 || certificate != null && certificate.length > 65536) {
            throw new IllegalArgumentException();
        }
        InetAddress address = literalAddress(host);
        if ("ipv6".equals(mode) && !(address instanceof Inet6Address)) {
            throw new IllegalArgumentException();
        }
        SSLSocketFactory tlsFactory = "tcp".equals(mode) ? tlsFactory(certificate) : null;
        Result result = new Result();
        result.nonce = UUID.randomUUID().toString().replace("-", "");
        try {
            if ("udp".equals(mode)) {
                byte[] payload = ("chaindesk-udp:" + result.nonce).getBytes(StandardCharsets.US_ASCII);
                try (DatagramSocket socket = new DatagramSocket()) {
                    socket.setSoTimeout(TIMEOUT_MS);
                    socket.send(new DatagramPacket(payload, payload.length, address, port));
                    result.sent = true;
                    result.success = true;
                }
                return result;
            }
            try (Socket socket = new Socket()) {
                socket.connect(new InetSocketAddress(address, port), TIMEOUT_MS);
                socket.setSoTimeout(TIMEOUT_MS);
                result.connected = true;
                if ("ipv6".equals(mode)) {
                    result.success = true;
                    return result;
                }
                try (SSLSocket tls = (SSLSocket) tlsFactory.createSocket(socket, TLS_HOST, port, true)) {
                    tls.setSoTimeout(TIMEOUT_MS);
                    SSLParameters parameters = tls.getSSLParameters();
                    parameters.setEndpointIdentificationAlgorithm("HTTPS");
                    parameters.setServerNames(Collections.singletonList(new SNIHostName(TLS_HOST)));
                    tls.setSSLParameters(parameters);
                    tls.startHandshake();
                    String request = "GET /a0?nonce=" + result.nonce + " HTTP/1.1\r\nHost: " + TLS_HOST + "\r\nConnection: close\r\n\r\n";
                    tls.getOutputStream().write(request.getBytes(StandardCharsets.US_ASCII));
                    tls.getOutputStream().flush();
                    verifyResponse(tls);
                    result.success = true;
                }
            }
        } catch (IOException failure) {
            result.exception = failure.getClass().getSimpleName();
        }
        return result;
    }

    static void verifyResponse(Socket socket) throws IOException {
        long deadline = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(TIMEOUT_MS);
        InputStream input = socket.getInputStream();
        ByteArrayOutputStream headerBytes = new ByteArrayOutputStream();
        int tail = 0;
        while (headerBytes.size() < 8192) {
            setReadDeadline(socket, deadline);
            int next = input.read();
            if (next < 0) {
                throw new IOException();
            }
            headerBytes.write(next);
            tail = (tail << 8) | next;
            if (tail == 0x0d0a0d0a) {
                break;
            }
        }
        if (tail != 0x0d0a0d0a) {
            throw new IOException();
        }
        String[] lines = new String(headerBytes.toByteArray(), StandardCharsets.US_ASCII).split("\r\n");
        String[] status = lines[0].split(" ", 3);
        if (status.length < 2 || !("HTTP/1.1".equals(status[0]) || "HTTP/1.0".equals(status[0])) || !"200".equals(status[1])) {
            throw new IOException();
        }
        int length = -1;
        for (int index = 1; index < lines.length; index++) {
            int separator = lines[index].indexOf(':');
            if (separator <= 0) {
                throw new IOException();
            }
            String name = lines[index].substring(0, separator).toLowerCase(Locale.ROOT);
            if ("transfer-encoding".equals(name)) {
                throw new IOException();
            }
            if ("content-length".equals(name)) {
                String value = lines[index].substring(separator + 1).trim();
                if (length >= 0 || !value.matches("[0-9]{1,4}")) {
                    throw new IOException();
                }
                length = Integer.parseInt(value);
            }
        }
        byte[] expected = "a0-chain-ok".getBytes(StandardCharsets.US_ASCII);
        if (length != expected.length) {
            throw new IOException();
        }
        byte[] body = new byte[length];
        int offset = 0;
        while (offset < length) {
            setReadDeadline(socket, deadline);
            int count = input.read(body, offset, length - offset);
            if (count < 0) {
                throw new IOException();
            }
            offset += count;
        }
        if (!java.util.Arrays.equals(body, expected)) {
            throw new IOException();
        }
    }

    private static void setReadDeadline(Socket socket, long deadline) throws IOException {
        long remaining = deadline - System.nanoTime();
        if (remaining <= 0) {
            throw new SocketTimeoutException();
        }
        socket.setSoTimeout((int) Math.max(1, TimeUnit.NANOSECONDS.toMillis(remaining)));
    }

    private static SSLSocketFactory tlsFactory(byte[] certificate) throws Exception {
        SSLContext context = SSLContext.getInstance("TLS");
        if (certificate == null) {
            context.init(null, null, null);
        } else {
            KeyStore trusted = KeyStore.getInstance(KeyStore.getDefaultType());
            trusted.load(null, null);
            trusted.setCertificateEntry("controlled-target", CertificateFactory.getInstance("X.509").generateCertificate(new ByteArrayInputStream(certificate)));
            TrustManagerFactory managers = TrustManagerFactory.getInstance(TrustManagerFactory.getDefaultAlgorithm());
            managers.init(trusted);
            context.init(null, managers.getTrustManagers(), null);
        }
        return context.getSocketFactory();
    }
}
