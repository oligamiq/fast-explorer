package dev.oligami.fastexplorer;

import android.content.Context;
import android.net.ConnectivityManager;
import android.net.LinkAddress;
import android.net.LinkProperties;
import android.net.Network;
import java.util.HashSet;
import java.util.Set;
import org.json.JSONArray;
import org.json.JSONObject;

final class FastExplorerNetwork {
    private FastExplorerNetwork() {}

    static String snapshot(Context context) {
        JSONArray interfaces = new JSONArray();
        try {
            Context applicationContext = context.getApplicationContext();
            Context stableContext = applicationContext != null ? applicationContext : context;
            ConnectivityManager manager = stableContext.getSystemService(ConnectivityManager.class);
            if (manager == null) return interfaces.toString();
            Set<String> seen = new HashSet<>();
            Network active = manager.getActiveNetwork();
            append(manager, active, interfaces, seen);
            for (Network network : manager.getAllNetworks()) {
                if (active != null && active.equals(network)) continue;
                append(manager, network, interfaces, seen);
            }
        } catch (Exception error) {
            android.util.Log.e("FastExplorer", "network interface snapshot failed", error);
        }
        return interfaces.toString();
    }

    private static void append(
            ConnectivityManager manager,
            Network network,
            JSONArray output,
            Set<String> seen) throws Exception {
        if (network == null) return;
        LinkProperties props = manager.getLinkProperties(network);
        if (props == null || props.getInterfaceName() == null || !seen.add(props.getInterfaceName())) {
            return;
        }
        JSONObject iface = new JSONObject();
        iface.put("name", props.getInterfaceName());
        iface.put("mtu", Math.max(props.getMtu(), 1280));
        JSONArray addresses = new JSONArray();
        for (LinkAddress address : props.getLinkAddresses()) {
            String host = address.getAddress().getHostAddress();
            if (host == null) continue;
            int zone = host.indexOf('%');
            if (zone >= 0) host = host.substring(0, zone);
            addresses.put(host + "/" + address.getPrefixLength());
        }
        iface.put("addresses", addresses);
        output.put(iface);
    }
}
