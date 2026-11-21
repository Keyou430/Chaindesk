package engine

import "testing"

func TestNativeSelfCheckValidatesBlockedConfiguration(t *testing.T) {
	result := NativeSelfCheck()
	if !result.ConfigValidated {
		t.Fatalf("blocked native probe configuration was rejected: %s", result.ErrorCode)
	}
}
