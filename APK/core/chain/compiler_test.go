package chain_test

import (
	"encoding/json"
	"reflect"
	"strings"
	"testing"

	"github.com/Keyou430/Chaindesk/APK/core/chain"
	"github.com/Keyou430/Chaindesk/APK/core/subscription"
)

func frontNode(t *testing.T) subscription.Node {
	t.Helper()
	result, err := subscription.Parse([]byte("proxies: [{name: DIRECT, type: ss, server: EXAMPLE.com., port: 443, cipher: aes-128-gcm, password: '密:@&*'}]\ntun: {enable: true}\nlisteners: [untrusted]\nrules: [MATCH,DIRECT]"))
	if err != nil {
		t.Fatal(err)
	}
	return result.Nodes[0]
}

func inputFor(t *testing.T) chain.Input {
	return chain.Input{Front: frontNode(t), Exit: chain.Endpoint{Host: " EXIT.example. ", Port: 1080, Username: "u:名", Password: "p\"&*"}, Preset: "allProxy"}
}

func TestCompileFixesTwoHopsWithoutSourceInheritance(t *testing.T) {
	input := inputFor(t)
	before, err := json.Marshal(input)
	if err != nil {
		t.Fatal(err)
	}
	result, err := chain.Compile(input)
	if err != nil {
		t.Fatal(err)
	}
	second, err := chain.Compile(input)
	if err != nil || result != second {
		t.Fatal("compiler is not deterministic")
	}
	after, _ := json.Marshal(input)
	if string(before) != string(after) {
		t.Fatal("compiler mutated caller-owned options")
	}
	if result.FrontName != "CHAIN_DESK_NODE_"+input.Front.ID || result.CapabilityVersion != 1 {
		t.Fatal("unsafe internal reference or capability version")
	}
	var config map[string]any
	if err := json.Unmarshal([]byte(result.Config), &config); err != nil {
		t.Fatal(err)
	}
	for _, key := range []string{"listeners", "mixed-port", "port", "socks-port", "external-controller", "external-ui", "script", "proxy-providers", "rule-providers"} {
		if _, exists := config[key]; exists {
			t.Fatalf("unwanted listener/control/inherited configuration: %s", key)
		}
	}
	if config["tun"].(map[string]any)["enable"] != false || config["ipv6"] != false || config["log-level"] != "silent" || config["geo-auto-update"] != false {
		t.Fatal("Android runtime boundaries were not preserved")
	}
	profile := config["profile"].(map[string]any)
	if profile["store-selected"] != false || profile["store-fake-ip"] != false {
		t.Fatal("runtime profile persistence enabled")
	}
	proxies := config["proxies"].([]any)
	if len(proxies) != 2 {
		t.Fatal("chain must contain exactly one front and one exit")
	}
	front, exit := proxies[0].(map[string]any), proxies[1].(map[string]any)
	if front["name"] != result.FrontName || front["udp"] != false || front["password"] != "密:@&*" {
		t.Fatal("front or Unicode secret changed")
	}
	if _, exists := front["dialer-proxy"]; exists {
		t.Fatal("front acquired a chain reference")
	}
	if exit["name"] != "CHAIN_DESK_FIXED_EXIT" || exit["type"] != "socks5" || exit["server"] != "exit.example" || exit["port"] != float64(1080) || exit["dialer-proxy"] != "CHAIN_DESK_FRONT" || exit["udp"] != false || exit["username"] != "u:名" || exit["password"] != "p\"&*" {
		t.Fatal("fixed exit or special-character authentication changed")
	}
	groups := config["proxy-groups"].([]any)
	if len(groups) != 2 {
		t.Fatal("unexpected fallback group")
	}
	for index, expected := range []struct{ name, child string }{{"CHAIN_DESK_FRONT", result.FrontName}, {"CHAIN_DESK_EXIT", "CHAIN_DESK_FIXED_EXIT"}} {
		group := groups[index].(map[string]any)
		if group["name"] != expected.name || group["type"] != "select" || !reflect.DeepEqual(group["proxies"], []any{expected.child}) {
			t.Fatal("group permits another exit, front-only or direct fallback")
		}
	}
	wantRules := []any{
		"NETWORK,udp,REJECT",
		"DOMAIN,www.gstatic.com,CHAIN_DESK_EXIT",
		"DOMAIN,cp.cloudflare.com,CHAIN_DESK_EXIT",
		"DOMAIN,api.ipify.org,CHAIN_DESK_EXIT",
		"DOMAIN,www.cloudflare.com,CHAIN_DESK_EXIT",
		"DOMAIN,localhost,DIRECT", "DOMAIN-SUFFIX,local,DIRECT",
		"IP-CIDR,127.0.0.0/8,DIRECT,no-resolve", "IP-CIDR,10.0.0.0/8,DIRECT,no-resolve",
		"IP-CIDR,172.16.0.0/12,DIRECT,no-resolve", "IP-CIDR,192.168.0.0/16,DIRECT,no-resolve",
		"IP-CIDR,169.254.0.0/16,DIRECT,no-resolve", "MATCH,CHAIN_DESK_EXIT",
	}
	if !reflect.DeepEqual(config["rules"], wantRules) {
		t.Fatal("UDP/probe pin/local exceptions/final exit rule order changed")
	}
	dns := config["dns"].(map[string]any)
	if dns["enable"] != true || dns["ipv6"] != false || dns["prefer-h3"] != false || !reflect.DeepEqual(dns["nameserver"], []any{"https://1.1.1.1/dns-query#CHAIN_DESK_EXIT"}) {
		t.Fatal("user DNS is not pinned to the full TCP chain")
	}
	if _, exists := dns["fallback"]; exists {
		t.Fatal("DNS fallback is forbidden")
	}
}

func TestCompileRejectsForgedNodesAndInvalidInputs(t *testing.T) {
	cases := []struct {
		name string
		edit func(*chain.Input)
		code string
	}{
		{"empty front", func(input *chain.Input) { input.Front = subscription.Node{} }, "NO_VALID_FRONT"},
		{"forged ID", func(input *chain.Input) { input.Front.ID = "DIRECT" }, "NO_VALID_FRONT"},
		{"forged metadata", func(input *chain.Input) { input.Front.Server = "other.example" }, "NO_VALID_FRONT"},
		{"forged authentication", func(input *chain.Input) { input.Front.Options["password"] = "" }, "NO_VALID_FRONT"},
		{"chain injection", func(input *chain.Input) { input.Front.Options["dialer-proxy"] = "DIRECT" }, "NO_VALID_FRONT"},
		{"TLS injection", func(input *chain.Input) { input.Front.Options["skip-cert-verify"] = true }, "NO_VALID_FRONT"},
		{"untrusted rejection", func(input *chain.Input) { input.Front.Rejection = "secret" }, "NO_VALID_FRONT"},
		{"same normalized endpoint", func(input *chain.Input) { input.Exit.Host, input.Exit.Port = " EXAMPLE.com. ", 443 }, "NO_VALID_FRONT"},
		{"exit URL", func(input *chain.Input) { input.Exit.Host = "https://secret.example" }, "EXIT_ADDRESS_INVALID"},
		{"IPv6 exit", func(input *chain.Input) { input.Exit.Host = "::1" }, "EXIT_ADDRESS_INVALID"},
		{"port zero", func(input *chain.Input) { input.Exit.Port = 0 }, "EXIT_ADDRESS_INVALID"},
		{"port overflow", func(input *chain.Input) { input.Exit.Port = 65536 }, "EXIT_ADDRESS_INVALID"},
		{"partial auth", func(input *chain.Input) { input.Exit.Password = "" }, "EXIT_AUTH_INVALID"},
		{"UTF8 auth bytes", func(input *chain.Input) { input.Exit.Username = strings.Repeat("名", 86) }, "EXIT_AUTH_INVALID"},
		{"malformed UTF8 auth", func(input *chain.Input) { input.Exit.Password = string([]byte{0xff}) }, "EXIT_AUTH_INVALID"},
		{"unsupported preset", func(input *chain.Input) { input.Preset = "secret" }, "RULE_PRESET_INVALID"},
		{"missing rules", func(input *chain.Input) { input.Preset = "chinaDirect" }, "RULE_RESOURCE_UNAVAILABLE"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			input := inputFor(t)
			tc.edit(&input)
			result, err := chain.Compile(input)
			if err == nil || err.Error() != tc.code || result.Config != "" {
				t.Fatalf("expected sanitized %s, got %v", tc.code, err)
			}
		})
	}
}

func TestCompileSupportsJSONRoundTripAndSocksAuthenticationBoundary(t *testing.T) {
	input := inputFor(t)
	input.Exit.Username, input.Exit.Password = strings.Repeat("名", 85), strings.Repeat("p", 255)
	encoded, err := json.Marshal(input)
	if err != nil {
		t.Fatal(err)
	}
	var decoded chain.Input
	if json.Unmarshal(encoded, &decoded) != nil {
		t.Fatal("DTO cannot round-trip JSON")
	}
	if _, err := chain.Compile(decoded); err != nil {
		t.Fatal(err)
	}
	input.Exit.Username, input.Exit.Password = "", ""
	result, err := chain.Compile(input)
	if err != nil {
		t.Fatal(err)
	}
	var config map[string]any
	json.Unmarshal([]byte(result.Config), &config)
	exit := config["proxies"].([]any)[1].(map[string]any)
	if _, exists := exit["username"]; exists {
		t.Fatal("empty authentication was emitted")
	}
	if _, exists := exit["password"]; exists {
		t.Fatal("empty authentication was emitted")
	}
}
