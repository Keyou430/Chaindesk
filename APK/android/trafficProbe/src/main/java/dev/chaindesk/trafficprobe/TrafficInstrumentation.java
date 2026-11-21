package dev.chaindesk.trafficprobe;

import android.app.Activity;
import android.app.Instrumentation;
import android.content.Context;
import android.net.ConnectivityManager;
import android.net.Network;
import android.net.NetworkCapabilities;
import android.os.Bundle;
import android.os.Process;
import java.util.Base64;

public final class TrafficInstrumentation extends Instrumentation {
    private Bundle arguments = new Bundle();

    @Override
    public void onCreate(Bundle input) {
        super.onCreate(input);
        arguments = input == null ? new Bundle() : new Bundle(input);
        start();
    }

    @Override
    public void onStart() {
        Bundle output = new Bundle();
        output.putString("mode", "invalid");
        output.putInt("uid", Process.myUid());
        output.putBoolean("success", false);
        output.putBoolean("expectationMet", false);
        boolean accepted = false;
        try {
            String mode = arguments.getString("mode");
            if (!("tcp".equals(mode) || "udp".equals(mode) || "ipv6".equals(mode))) {
                throw new IllegalArgumentException();
            }
            output.putString("mode", mode);
            boolean requireVpn = argumentBoolean("requireVpn");
            boolean expectBlocked = argumentBoolean("expectBlocked");
            output.putBoolean("expectBlocked", expectBlocked);
            ConnectivityManager connectivity = (ConnectivityManager) getTargetContext().getSystemService(Context.CONNECTIVITY_SERVICE);
            Network network = connectivity.getActiveNetwork();
            NetworkCapabilities capabilities = network == null ? null : connectivity.getNetworkCapabilities(network);
            boolean vpnActive = capabilities != null && capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN);
            output.putBoolean("vpnActive", vpnActive);
            if (requireVpn && !vpnActive) {
                throw new IllegalStateException();
            }
            String encodedCertificate = arguments.getString("certificate");
            byte[] certificate = null;
            if (encodedCertificate != null) {
                if (encodedCertificate.length() > 90000) {
                    throw new IllegalArgumentException();
                }
                certificate = Base64.getDecoder().decode(encodedCertificate);
            }
            TrafficClient.Result result = TrafficClient.run(mode, arguments.getString("host"), Integer.parseInt(arguments.getString("port", "0")), certificate);
            output.putBoolean("success", result.success);
            output.putString("exception", result.exception);
            if (!"ipv6".equals(mode)) {
                output.putString("nonce", result.nonce);
            }
            if ("udp".equals(mode)) {
                output.putBoolean("sent", result.sent);
                output.remove("expectationMet");
                accepted = result.sent;
            } else {
                output.putBoolean("connected", result.connected);
                accepted = expectBlocked ? !result.connected && !result.exception.isEmpty() : result.success;
                output.putBoolean("expectationMet", accepted);
            }
        } catch (Exception failure) {
            output.putString("exception", failure.getClass().getSimpleName());
        }
        finish(accepted ? Activity.RESULT_OK : Activity.RESULT_CANCELED, output);
    }

    private boolean argumentBoolean(String name) {
        String value = arguments.getString(name, "false");
        if (!("true".equals(value) || "false".equals(value))) {
            throw new IllegalArgumentException();
        }
        return "true".equals(value);
    }
}
