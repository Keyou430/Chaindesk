package engine

import (
	"strings"
	"testing"

	"github.com/metacubex/mihomo/constant/provider"
)

const validFixture = `{"front":{"server":"127.0.0.1","port":18388,"cipher":"aes-128-gcm","password":"fixture-front-secret"},"exit":{"server":"127.0.0.1","port":18080,"username":"fixture-user","password":"fixture-exit-secret"}}`

func TestFixtureRejectsUnsafeOrAmbiguousInput(t *testing.T) {
	for _, input := range []string{
		`{}`, validFixture + `{}`,
		strings.Replace(validFixture, `"port":18388`, `"port":0`, 1),
		strings.Replace(validFixture, `"port":18080`, `"port":65536`, 1),
		strings.Replace(validFixture, `127.0.0.1`, `::1`, 1),
		strings.Replace(validFixture, `127.0.0.1`, `front.example`, 1),
		strings.Replace(validFixture, `aes-128-gcm`, `none`, 1),
		strings.Replace(validFixture, `fixture-user`, ``, 1),
		strings.Replace(validFixture, `fixture-exit-secret`, ``, 1),
		strings.Replace(validFixture, `fixture-user`, strings.Repeat("u", 256), 1),
		strings.Replace(validFixture, `fixture-exit-secret`, strings.Repeat("p", 256), 1),
		strings.Replace(validFixture, `"front":`, `"rules":[],"front":`, 1),
		strings.Repeat(" ", 16*1024+1),
	} {
		if _, err := parseFixture([]byte(input)); err == nil {
			t.Fatal("unsafe fixture accepted")
		} else if strings.Contains(err.Error(), "secret") {
			t.Fatal("fixture error exposed credentials")
		}
	}
}

func TestFixtureHasOnlyFixedChainRouting(t *testing.T) {
	cfg, err := parseFixture([]byte(validFixture))
	if err != nil {
		t.Fatal(err)
	}
	if cfg.General.Tun.Enable || cfg.General.MixedPort != 0 || cfg.General.Port != 0 || cfg.General.SocksPort != 0 || cfg.Controller.ExternalController != "" {
		t.Fatal("fixture enabled unmanaged listeners")
	}
	if len(cfg.Rules) != 2 || cfg.Rules[0].Adapter() != "REJECT" || cfg.Rules[1].Adapter() != "CHAIN-EXIT" {
		t.Fatal("fixture permits an unexpected route")
	}
	if !cfg.DNS.Enable || cfg.General.IPv6 || len(cfg.Providers) != 1 || cfg.Providers["default"].VehicleType() != provider.Compatible || len(cfg.RuleProviders) != 0 {
		t.Fatalf("fixture policy: DNS=%v IPv6=%v providers=%v ruleProviders=%v", cfg.DNS.Enable, cfg.General.IPv6, cfg.Providers, cfg.RuleProviders)
	}
}
