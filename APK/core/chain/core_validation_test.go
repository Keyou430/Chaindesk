package chain_test

import (
	"net/netip"
	"testing"

	"github.com/Keyou430/Chaindesk/APK/core/chain"
	"github.com/Keyou430/Chaindesk/APK/core/subscription"
	C "github.com/metacubex/mihomo/constant"
	"github.com/metacubex/mihomo/hub/executor"
)

func TestCandidateConfigsParseInPinnedCoreAndApplyRoutingBoundaries(t *testing.T) {
	uuid := "11111111-1111-1111-1111-111111111111"
	candidates := []string{
		"type: ss, cipher: aes-128-gcm, password: fixture",
		"type: ss, cipher: aes-256-gcm, password: fixture",
		"type: ss, cipher: chacha20-ietf-poly1305, password: fixture",
		"type: vmess, uuid: " + uuid + ", alterId: 0, cipher: auto, network: tcp",
		"type: vmess, uuid: " + uuid + ", alterId: 0, cipher: auto, tls: true, servername: front.example, alpn: [h2, http/1.1]",
		"type: vless, uuid: " + uuid + ", tls: true, flow: '', network: tcp, servername: front.example",
		"type: trojan, password: fixture, sni: front.example",
		"type: anytls, password: fixture, sni: front.example",
	}
	for _, fields := range candidates {
		imported, err := subscription.Parse([]byte("proxies: [{server: 192.0.2.1, port: 443, " + fields + "}]"))
		if err != nil {
			t.Fatal(err)
		}
		compiled, err := chain.Compile(chain.Input{Front: imported.Nodes[0], Exit: chain.Endpoint{Host: "192.0.2.2", Port: 1080}, Preset: "allProxy"})
		if err != nil {
			t.Fatal(err)
		}
		parsed, err := executor.ParseWithBytes([]byte(compiled.Config))
		if err != nil {
			t.Fatalf("pinned core rejected candidate %s: %v", imported.Nodes[0].Protocol, err)
		}
		for _, proxy := range parsed.Proxies {
			defer proxy.Close()
		}
		if parsed.General.Port != 0 || parsed.General.SocksPort != 0 || parsed.General.MixedPort != 0 || parsed.General.RedirPort != 0 || parsed.General.TProxyPort != 0 || parsed.General.Tun.Enable || parsed.DNS.Listen != "" || len(parsed.Listeners) != 0 {
			t.Fatal("parsed configuration opens an unmanaged listener")
		}
		for _, tc := range []struct {
			host, address string
			network       C.NetWork
			want          string
		}{
			{"api.ipify.org", "127.0.0.1", C.TCP, "CHAIN_DESK_EXIT"},
			{"cp.cloudflare.com", "192.168.1.2", C.TCP, "CHAIN_DESK_EXIT"},
			{"www.gstatic.com", "10.0.0.1", C.TCP, "CHAIN_DESK_EXIT"},
			{"www.cloudflare.com", "169.254.1.1", C.TCP, "CHAIN_DESK_EXIT"},
			{"localhost", "127.0.0.1", C.TCP, "DIRECT"},
			{"printer.local", "192.168.1.2", C.TCP, "DIRECT"},
			{"", "172.16.0.1", C.TCP, "DIRECT"},
			{"target.example", "198.51.100.1", C.TCP, "CHAIN_DESK_EXIT"},
			{"target.example", "198.51.100.1", C.UDP, "REJECT"},
			{"printer.local", "192.168.1.2", C.UDP, "REJECT"},
		} {
			metadata := &C.Metadata{Host: tc.host, DstIP: netip.MustParseAddr(tc.address), DstPort: 443, NetWork: tc.network}
			matched := ""
			for _, rule := range parsed.Rules {
				if ok, adapter := rule.Match(metadata, C.RuleMatchHelper{}); ok {
					matched = adapter
					break
				}
			}
			if matched != tc.want {
				t.Fatalf("%s/%s: got %s, want %s", tc.host, tc.network, matched, tc.want)
			}
		}
	}
}
