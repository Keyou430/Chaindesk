package engine

import (
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"io"
	"os"
	"path/filepath"

	"github.com/Keyou430/Chaindesk/APK/core/chain"
	C "github.com/metacubex/mihomo/constant"
)

var rulesDirectory string

func SetRulesDirectory(path string) error {
	coreMutex.Lock()
	defer coreMutex.Unlock()
	if session.id != 0 {
		return errors.New("CORE_SESSION_ACTIVE")
	}
	if !filepath.IsAbs(path) || verifyRuleFiles(path) != nil {
		return errors.New("RULE_RESOURCE_UNAVAILABLE")
	}
	rulesDirectory = path
	C.SetHomeDir(path)
	return nil
}

func verifyRuleFiles(path string) error {
	invalid := errors.New("RULE_RESOURCE_UNAVAILABLE")
	if path == "" {
		return invalid
	}
	for _, expected := range chain.RuleFiles() {
		file, err := os.Open(filepath.Join(path, expected.Name))
		if err != nil {
			return invalid
		}
		stat, err := file.Stat()
		if err != nil || !stat.Mode().IsRegular() || stat.Size() != expected.Bytes {
			file.Close()
			return invalid
		}
		hash := sha256.New()
		_, err = io.Copy(hash, io.LimitReader(file, expected.Bytes+1))
		file.Close()
		if err != nil || hex.EncodeToString(hash.Sum(nil)) != expected.SHA256 {
			return invalid
		}
	}
	return nil
}

func verifyPresetResources(preset string) error {
	if preset == "chinaDirect" {
		return verifyRuleFiles(rulesDirectory)
	}
	return nil
}
