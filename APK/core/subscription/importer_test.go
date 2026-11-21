package subscription_test

import (
	"os"
	"strings"
	"testing"
	"unicode/utf8"

	"github.com/Keyou430/Chaindesk/APK/core/subscription"
)

func TestDesktopIdentityExcludesNameAndOrderButPreservesSecrets(t *testing.T) {
	source, err := os.ReadFile("../../testFixtures/subscription/desktop-semantic.yaml")
	if err != nil {
		t.Fatal(err)
	}
	result, err := subscription.Parse(source)
	if err != nil {
		t.Fatal(err)
	}
	if result.CapabilityVersion != 1 || len(result.Nodes) != 3 || result.Duplicates != 1 {
		t.Fatalf("unexpected import summary: version=%d nodes=%d duplicates=%d", result.CapabilityVersion, len(result.Nodes), result.Duplicates)
	}
	first := result.Nodes[0]
	if first.Server != "example.com" || first.Rejection != "" || first.Options["password"] != "first" {
		t.Fatal("node normalization or authentication changed")
	}
	// SHA-256 of desktop serde_json [1, sorted [key,value] pairs].
	if first.ID != "1f04287d3cdea910918197a2352b852fef46cac0b458e891b578faa30132fcf2" {
		t.Fatalf("desktop identity mismatch: %s", first.ID)
	}
	if result.Nodes[1].ID == first.ID || result.Nodes[2].Server != "xn--bcher-kva.example" {
		t.Fatal("different authentication collapsed or IDNA normalization changed")
	}
	if len(first.Options) != 5 {
		t.Fatal("subscription fields escaped the option whitelist")
	}
}

func TestRejectsUnsafeYAMLWithoutEchoingSource(t *testing.T) {
	cases := []string{
		"proxies: &secret []\nx: *secret",
		"proxies: !!seq []",
		"proxies: !secret []",
		"%TAG !e! tag:secret.example,2026:\n---\nproxies: []",
		"proxies: []\n---\nproxies: []",
		"proxies: []\nproxies: []",
		"proxies: []\nx: {secret: 1, secret: 2}",
		"proxies: [secret]",
		"proxies: {}",
		"proxy-providers: {secret: {url: https://secret.example}}",
		"proxies: []\nx: {1: secret}",
		"proxies: []\nx: " + strings.Repeat("[", 32) + "0" + strings.Repeat("]", 32),
		"proxies: [",
		string([]byte{'p', ':', 0xff}),
	}
	for _, source := range cases {
		_, err := subscription.Parse([]byte(source))
		if err == nil || err.Error() != "SUBSCRIPTION_UNSUPPORTED" {
			t.Fatalf("expected sanitized rejection, got %v", err)
		}
	}
}

func TestAcceptsExactLimitsAndRejectsFirstExcess(t *testing.T) {
	source := "proxies: []\n#" + strings.Repeat("x", 10*1024*1024-len("proxies: []\n#"))
	if _, err := subscription.Parse([]byte(source)); err != nil {
		t.Fatal(err)
	}
	if _, err := subscription.Parse([]byte(source + "x")); err == nil || err.Error() != "SUBSCRIPTION_TOO_LARGE" {
		t.Fatal("size limit not enforced")
	}
	node := "- {type: ss, server: example.com, port: 443, cipher: aes-128-gcm, password: fixture}\n"
	source = "proxies:\n" + strings.Repeat(node, 5000)
	result, err := subscription.Parse([]byte(source))
	if err != nil || len(result.Nodes) != 1 || result.Duplicates != 4999 {
		t.Fatalf("node boundary or duplicate accounting failed: %v", err)
	}
	if _, err := subscription.Parse([]byte(source + node)); err == nil || err.Error() != "SUBSCRIPTION_TOO_LARGE" {
		t.Fatal("node limit not enforced")
	}
	source = "proxies: []\nx: " + strings.Repeat("[", 31) + "0" + strings.Repeat("]", 31)
	if _, err := subscription.Parse([]byte(source)); err != nil {
		t.Fatalf("exact nesting boundary rejected: %v", err)
	}
}

func TestClassifiesUnsupportedAndMalformedCandidates(t *testing.T) {
	uuid := "11111111-1111-1111-1111-111111111111"
	cases := []struct{ fields, rejection string }{
		{"type: ss, cipher: aes-128-gcm, password: fixture", ""},
		{"type: ss, cipher: aes-256-gcm, password: fixture", ""},
		{"type: ss, cipher: chacha20-ietf-poly1305, password: fixture", ""},
		{"type: vmess, uuid: " + uuid + ", alterId: 0, cipher: auto", ""},
		{"type: vmess, uuid: " + uuid + ", tls: true", ""},
		{"type: vless, uuid: " + uuid + ", tls: true", ""},
		{"type: trojan, password: fixture", ""},
		{"type: anytls, password: fixture", ""},
		{"type: ss, cipher: none, password: fixture", "FRONT_CIPHER_UNSUPPORTED"},
		{"type: ss, cipher: aes-128-gcm, password: fixture, dialer-proxy: DIRECT", "FRONT_CHAIN_UNSUPPORTED"},
		{"type: trojan, password: fixture, skip-cert-verify: true", "FRONT_TLS_INSECURE"},
		{"type: vless, uuid: " + uuid + ", tls: false", "FRONT_TLS_INSECURE"},
		{"type: vmess, uuid: " + uuid + ", cipher: none", "FRONT_CIPHER_UNSUPPORTED"},
		{"type: vmess, uuid: " + uuid + ", alterId: 1", "FRONT_TRANSPORT_UNSUPPORTED"},
		{"type: vless, uuid: invalid, tls: true", "FRONT_AUTH_INVALID"},
		{"type: vless, uuid: " + uuid + ", tls: true, flow: vision", "FRONT_TRANSPORT_UNSUPPORTED"},
		{"type: ss, password: fixture, cipher: aes-128-gcm, network: ws", "FRONT_TRANSPORT_UNSUPPORTED"},
		{"type: ss, password: fixture, cipher: aes-128-gcm, plugin: obfs", "FRONT_TRANSPORT_UNSUPPORTED"},
		{"type: vless, uuid: " + uuid + ", tls: true, reality-opts: {}", "FRONT_TRANSPORT_UNSUPPORTED"},
		{"type: trojan, password: 123", "FRONT_FIELD_INVALID"},
		{"type: trojan, password: ''", "FRONT_AUTH_INVALID"},
		{"type: trojan, password: fixture, tls: 'true'", "FRONT_FIELD_INVALID"},
		{"type: trojan, password: fixture, sni: {}", "FRONT_FIELD_INVALID"},
		{"type: trojan, password: fixture, skip-cert-verify: secret", "FRONT_FIELD_INVALID"},
		{"type: trojan, password: fixture, alpn: [1]", "FRONT_FIELD_INVALID"},
		{"type: trojan, password: fixture, alpn: h2", "FRONT_FIELD_INVALID"},
		{"type: trojan, password: fixture, alpn: true", "FRONT_FIELD_INVALID"},
		{"type: trojan, password: fixture, flow: 1", "FRONT_FIELD_INVALID"},
		{"type: tuic, password: fixture", "FRONT_PROTOCOL_UNSUPPORTED"},
	}
	for _, tc := range cases {
		result, err := subscription.Parse([]byte("proxies: [{server: example.com, port: 443, " + tc.fields + "}]"))
		if err != nil {
			t.Fatal(err)
		}
		if result.Nodes[0].Rejection != tc.rejection {
			t.Fatalf("fields %q: got %q, want %q", tc.fields, result.Nodes[0].Rejection, tc.rejection)
		}
	}
}

func TestHostRejectionAndUnicodeNames(t *testing.T) {
	for _, host := range []string{"https://example.com", "::1", "999.1.2.3", "a_b.test", "example.com:443", "a/b", ""} {
		result, err := subscription.Parse([]byte("proxies: [{type: ss, server: '" + host + "', port: 443, cipher: aes-128-gcm, password: fixture}]"))
		if err != nil || result.Nodes[0].Rejection != "FRONT_ENDPOINT_INVALID" {
			t.Fatalf("invalid host accepted: %q (%v)", host, err)
		}
	}
	source := "proxies: [{name: '" + strings.Repeat("中😀", 200) + "', type: ss, server: example.com, port: 443, cipher: aes-128-gcm, password: '密<>&😀\u2028'}]"
	result, err := subscription.Parse([]byte(source))
	if err != nil || utf8.RuneCountInString(result.Nodes[0].Name) != 256 || result.Nodes[0].Rejection != "" {
		t.Fatalf("Unicode name handling failed: %v", err)
	}
	if result.Nodes[0].ID != "ae15770d9200b2f730f71275bccc457137b83ffb0cfcf99ff5c78802067bc259" {
		t.Fatalf("Unicode JSON identity mismatch: %s", result.Nodes[0].ID)
	}
}

func TestRejectsYAMLFloatingPointPortAndAlterID(t *testing.T) {
	for _, fields := range []string{
		"type: ss, port: 443.0, cipher: aes-128-gcm, password: fixture",
		"type: vmess, port: 443, uuid: 11111111-1111-1111-1111-111111111111, alterId: 0.0",
	} {
		result, err := subscription.Parse([]byte("proxies: [{server: example.com, " + fields + "}]"))
		if err != nil || result.Nodes[0].Rejection == "" {
			t.Fatalf("YAML floating-point integer field accepted: %v", err)
		}
	}
}

func TestNormalizesDesktopURLHostForms(t *testing.T) {
	for _, tc := range []struct{ input, want string }{
		{" EXAMPLE.com. ", "example.com"},
		{"bücher.example", "xn--bcher-kva.example"},
		{"%65xample.com", "example.com"},
		{"0x7f.1", "127.0.0.1"},
		{"０x７f.１", "127.0.0.1"},
		{"%31%32%37.1", "127.0.0.1"},
	} {
		got, err := subscription.NormalizeHost(tc.input)
		if err != nil || got != tc.want {
			t.Fatalf("host %q: got %q (%v), want %q", tc.input, got, err, tc.want)
		}
	}
	for _, host := range []string{"example.123", "127.1", "0127.0.0.1", "0x100000000", "0xffffffffffffffffffff", "%2fexample.com", "0x7f.0x100.1"} {
		if _, err := subscription.NormalizeHost(host); err == nil {
			t.Fatalf("ambiguous or invalid URL host accepted: %q", host)
		}
	}
}
