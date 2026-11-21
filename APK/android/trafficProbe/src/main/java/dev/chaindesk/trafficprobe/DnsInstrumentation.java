package dev.chaindesk.trafficprobe;

import android.app.Activity;
import android.app.Instrumentation;
import android.net.ConnectivityManager;
import android.net.NetworkCapabilities;
import android.os.Bundle;
import android.os.Process;
import java.io.ByteArrayOutputStream;
import java.io.DataOutputStream;
import java.net.DatagramPacket;
import java.net.DatagramSocket;
import java.net.InetAddress;
import java.nio.charset.StandardCharsets;
import java.security.SecureRandom;
import java.util.UUID;

public final class DnsInstrumentation extends Instrumentation {
    @Override
    public void onCreate(Bundle arguments) {
        super.onCreate(arguments);
        start();
    }

    @Override
    public void onStart() {
        Bundle output = new Bundle();
        output.putBoolean("success", false);
        output.putInt("uid", Process.myUid());
        boolean success = false;
        try {
            ConnectivityManager connectivity = getTargetContext().getSystemService(ConnectivityManager.class);
            NetworkCapabilities capabilities = connectivity.getNetworkCapabilities(connectivity.getActiveNetwork());
            boolean vpn = capabilities != null && capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN);
            output.putBoolean("vpnActive", vpn);
            if (!vpn) throw new IllegalStateException();
            int id = new SecureRandom().nextInt(65536);
            ByteArrayOutputStream bytes = new ByteArrayOutputStream();
            DataOutputStream query = new DataOutputStream(bytes);
            query.writeShort(id);
            query.writeShort(0x0100);
            query.writeShort(1);
            query.writeShort(0);
            query.writeShort(0);
            query.writeShort(0);
            String domain = "t" + UUID.randomUUID().toString().replace("-", "") + ".example.com";
            for (String label : domain.split("\\.")) {
                byte[] encoded = label.getBytes(StandardCharsets.US_ASCII);
                query.writeByte(encoded.length);
                query.write(encoded);
            }
            query.writeByte(0);
            query.writeShort(1);
            query.writeShort(1);
            byte[] request = bytes.toByteArray();
            byte[] response = new byte[4096];
            try (DatagramSocket socket = new DatagramSocket()) {
                socket.connect(InetAddress.getByAddress(new byte[] {(byte) 172, 29, 0, 2}), 53);
                socket.setSoTimeout(5000);
                socket.send(new DatagramPacket(request, request.length));
                DatagramPacket packet = new DatagramPacket(response, response.length);
                socket.receive(packet);
                int flags = unsignedShort(response, 2);
                int rcode = flags & 15;
                if (packet.getLength() < request.length || unsignedShort(response, 0) != id || (flags & 0xfa00) != 0x8000 ||
                        unsignedShort(response, 4) != 1 || !(rcode == 0 || rcode == 3)) throw new IllegalStateException();
                for (int index = 12; index < request.length; index++) {
                    if (response[index] != request[index]) throw new IllegalStateException();
                }
                output.putInt("rcode", rcode);
                output.putInt("responseBytes", packet.getLength());
            }
            success = true;
            output.putBoolean("success", true);
        } catch (Exception failure) {
            output.putString("exception", failure.getClass().getSimpleName());
        }
        finish(success ? Activity.RESULT_OK : Activity.RESULT_CANCELED, output);
    }

    private static int unsignedShort(byte[] value, int offset) {
        return (value[offset] & 255) << 8 | value[offset + 1] & 255;
    }
}
