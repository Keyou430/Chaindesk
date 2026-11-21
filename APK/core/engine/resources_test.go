package engine

import (
	"os"
	"path/filepath"
	"testing"

	C "github.com/metacubex/mihomo/constant"
)

func TestRuleResourceSetupRejectsMissingAndCorruptFilesBeforeChangingHome(t *testing.T) {
	original := C.Path.HomeDir()
	path := t.TempDir()
	if err := SetRulesDirectory(path); err == nil || err.Error() != "RULE_RESOURCE_UNAVAILABLE" {
		t.Fatal("missing DAT accepted", err)
	}
	if C.Path.HomeDir() != original {
		t.Fatal("failed verification changed native home")
	}
	for _, name := range []string{"geoip.dat", "geosite.dat"} {
		if err := os.WriteFile(filepath.Join(path, name), []byte("untrusted"), 0600); err != nil {
			t.Fatal(err)
		}
	}
	if err := SetRulesDirectory(path); err == nil || err.Error() != "RULE_RESOURCE_UNAVAILABLE" {
		t.Fatal("corrupt DAT accepted", err)
	}
	if C.Path.HomeDir() != original {
		t.Fatal("corrupt verification changed native home")
	}
}
