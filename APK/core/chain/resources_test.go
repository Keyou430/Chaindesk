package chain_test

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"net/netip"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/Keyou430/Chaindesk/APK/core/chain"
	"github.com/metacubex/mihomo/component/geodata"
	C "github.com/metacubex/mihomo/constant"
	"github.com/metacubex/mihomo/hub/executor"
)

func TestChinaDirectRequiresExactBundledRuleVersion(t *testing.T) {
	input := inputFor(t)
	input.Preset = "chinaDirect"
	for _, version := range []string{"", "latest", chain.RuleVersion + "-modified"} {
		input.RuleVersion = version
		if result, err := chain.Compile(input); err == nil || err.Error() != "RULE_RESOURCE_UNAVAILABLE" || result.Config != "" {
			t.Fatal("unavailable or mismatched rule version did not fail closed")
		}
	}
	input.RuleVersion = chain.RuleVersion
	result, err := chain.Compile(input)
	if err != nil {
		t.Fatal(err)
	}
	var config map[string]any
	if json.Unmarshal([]byte(result.Config), &config) != nil {
		t.Fatal("compiler did not return JSON")
	}
	rules := config["rules"].([]any)
	if len(rules) != 15 || !reflect.DeepEqual(rules[12:], []any{"GEOSITE,cn,DIRECT", "GEOIP,CN,DIRECT,no-resolve", "MATCH,CHAIN_DESK_EXIT"}) {
		t.Fatal("China exceptions do not precede the fixed final exit")
	}
	if !reflect.DeepEqual(rules[:5], []any{"NETWORK,udp,REJECT", "DOMAIN,www.gstatic.com,CHAIN_DESK_EXIT", "DOMAIN,cp.cloudflare.com,CHAIN_DESK_EXIT", "DOMAIN,api.ipify.org,CHAIN_DESK_EXIT", "DOMAIN,www.cloudflare.com,CHAIN_DESK_EXIT"}) {
		t.Fatal("probe pins or UDP guard were displaced by China exceptions")
	}
	if config["geodata-mode"] != true || config["geo-auto-update"] != false {
		t.Fatal("bundled DAT mode or no-download policy changed")
	}
	dns := config["dns"].(map[string]any)
	if !reflect.DeepEqual(dns["direct-nameserver"], []any{"https://dns.alidns.com/dns-query#DIRECT"}) ||
		!reflect.DeepEqual(dns["nameserver-policy"], map[string]any{"geosite:cn": []any{"https://dns.alidns.com/dns-query#DIRECT"}}) ||
		!reflect.DeepEqual(dns["nameserver"], []any{"https://1.1.1.1/dns-query#CHAIN_DESK_EXIT"}) {
		t.Fatal("direct DNS policy escaped China rules or replaced chain DNS")
	}
}

func TestChinaDirectParsesBundledResourcesAndPinsProbesBeforeChinaRules(t *testing.T) {
	runtimeDirectory := t.TempDir()
	for _, file := range chain.RuleFiles() {
		data, err := os.ReadFile(filepath.Join("../../android/app/src/main/assets/rules", file.Name))
		if err != nil {
			t.Fatal(err)
		}
		digest := sha256.Sum256(data)
		if int64(len(data)) != file.Bytes || hex.EncodeToString(digest[:]) != file.SHA256 {
			t.Fatal("unverified geodata cannot enter the isolated parser test")
		}
		if err := os.WriteFile(filepath.Join(runtimeDirectory, file.Name), data, 0o600); err != nil {
			t.Fatal(err)
		}
	}
	previousHome := C.Path.HomeDir()
	previousGeodataMode := geodata.GeodataMode()
	C.SetHomeDir(runtimeDirectory)
	geodata.ClearGeoSiteCache()
	geodata.ClearGeoIPCache()
	t.Cleanup(func() {
		geodata.ClearGeoSiteCache()
		geodata.ClearGeoIPCache()
		C.SetHomeDir(previousHome)
		geodata.SetGeodataMode(previousGeodataMode)
	})
	input := inputFor(t)
	input.Preset, input.RuleVersion = "chinaDirect", chain.RuleVersion
	compiled, err := chain.Compile(input)
	if err != nil {
		t.Fatal(err)
	}
	parsed, err := executor.ParseWithBytes([]byte(compiled.Config))
	if err != nil {
		t.Fatal(err)
	}
	geodata.SetGeodataMode(parsed.General.GeodataMode)
	for _, proxy := range parsed.Proxies {
		defer proxy.Close()
	}
	for _, tc := range []struct{ host, address, want string }{
		{"baidu.com", "1.1.1.1", "DIRECT"},
		{"target.example", "114.114.114.114", "DIRECT"},
		{"cp.cloudflare.com", "114.114.114.114", "CHAIN_DESK_EXIT"},
		{"target.example", "1.1.1.1", "CHAIN_DESK_EXIT"},
	} {
		metadata := &C.Metadata{Host: tc.host, DstIP: netip.MustParseAddr(tc.address), DstPort: 443, NetWork: C.TCP}
		matched := ""
		for _, rule := range parsed.Rules {
			if ok, adapter := rule.Match(metadata, C.RuleMatchHelper{}); ok {
				matched = adapter
				break
			}
		}
		if matched != tc.want {
			t.Fatalf("%s/%s: got %s, want %s", tc.host, tc.address, matched, tc.want)
		}
	}
}

func TestBundledRuleMetadataMatchesAssetsAndCannotBeMutated(t *testing.T) {
	files := chain.RuleFiles()
	if len(files) != 2 {
		t.Fatal("missing required bundled geodata")
	}
	var manifest struct {
		RuleVersion string           `json:"ruleVersion"`
		Files       []chain.RuleFile `json:"files"`
	}
	data, err := os.ReadFile("../../android/app/src/main/assets/rules/manifest.json")
	if err != nil || json.Unmarshal(data, &manifest) != nil {
		t.Fatalf("rule manifest is unavailable: %v", err)
	}
	if manifest.RuleVersion != chain.RuleVersion || !reflect.DeepEqual(manifest.Files, files) {
		t.Fatal("packaged manifest differs from immutable core expectations")
	}
	for _, file := range files {
		data, err := os.ReadFile(filepath.Join("../../android/app/src/main/assets/rules", file.Name))
		if err != nil {
			t.Fatal(err)
		}
		digest := sha256.Sum256(data)
		if int64(len(data)) != file.Bytes || hex.EncodeToString(digest[:]) != file.SHA256 {
			t.Fatalf("packaged asset checksum or length mismatch: %s", file.Name)
		}
	}
	files[0].Name, files[0].SHA256, files[0].Bytes = "modified", "modified", 0
	if !reflect.DeepEqual(chain.RuleFiles(), manifest.Files) {
		t.Fatal("caller mutated the runtime resource policy")
	}
}
