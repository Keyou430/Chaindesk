package engine

import (
	"bytes"
	"encoding/json"
	"errors"
	"io"
	"net/netip"

	"github.com/Keyou430/Chaindesk/APK/core/chain"
	"github.com/metacubex/mihomo/config"
	"github.com/metacubex/mihomo/hub/executor"
)

type runtimeChainInput struct {
	Input     chain.Input `json:"input"`
	FrontIPv4 string      `json:"frontIpv4"`
	ExitIPv4  string      `json:"exitIpv4"`
}

func decodeChainInput(data []byte, output any) error {
	if len(data) == 0 || len(data) > 128*1024 {
		return errors.New("CHAIN_INPUT_INVALID")
	}
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if decoder.Decode(output) != nil || decoder.Decode(new(any)) != io.EOF {
		return errors.New("CHAIN_INPUT_INVALID")
	}
	return nil
}

func CompileChain(data []byte) (chain.Result, error) {
	coreMutex.Lock()
	defer coreMutex.Unlock()
	if session.id != 0 {
		return chain.Result{}, errors.New("CORE_SESSION_ACTIVE")
	}
	var input chain.Input
	if err := decodeChainInput(data, &input); err != nil {
		return chain.Result{}, err
	}
	compiled, err := chain.Compile(input)
	if err != nil {
		return chain.Result{}, err
	}
	if err := verifyPresetResources(input.Preset); err != nil {
		return chain.Result{}, err
	}
	if _, err := executor.ParseWithBytes([]byte(compiled.Config)); err != nil {
		return chain.Result{}, errors.New("CORE_CONFIG_INVALID")
	}
	return compiled, nil
}

func parseRuntimeChain(data []byte) (*config.Config, string, error) {
	var prepared runtimeChainInput
	if err := decodeChainInput(data, &prepared); err != nil {
		return nil, "", err
	}
	compiled, err := chain.Compile(prepared.Input)
	if err != nil {
		return nil, "", err
	}
	if err := verifyPresetResources(prepared.Input.Preset); err != nil {
		return nil, "", err
	}
	addresses := make([]string, 2)
	for index, host := range []struct{ original, resolved string }{
		{prepared.Input.Front.Server, prepared.FrontIPv4}, {prepared.Input.Exit.Host, prepared.ExitIPv4},
	} {
		value := host.resolved
		if value == "" {
			value = host.original
		}
		address, err := netip.ParseAddr(value)
		if err != nil || !address.Is4() || address.IsUnspecified() || address.IsMulticast() {
			return nil, "", errors.New("BOOTSTRAP_REQUIRED")
		}
		addresses[index] = address.String()
	}
	if addresses[0] == addresses[1] && prepared.Input.Front.Port == prepared.Input.Exit.Port {
		return nil, "", errors.New("NO_VALID_FRONT")
	}
	var document map[string]any
	if json.Unmarshal([]byte(compiled.Config), &document) != nil {
		return nil, "", errors.New("CORE_CONFIG_INVALID")
	}
	proxies := document["proxies"].([]any)
	front := proxies[0].(map[string]any)
	if _, err := netip.ParseAddr(prepared.Input.Front.Server); err != nil {
		switch prepared.Input.Front.Protocol {
		case "vmess", "vless":
			if front["servername"] == nil || front["servername"] == "" {
				front["servername"] = prepared.Input.Front.Server
			}
		case "trojan", "anytls":
			if front["sni"] == nil || front["sni"] == "" {
				front["sni"] = prepared.Input.Front.Server
			}
		}
	}
	front["server"] = addresses[0]
	proxies[1].(map[string]any)["server"] = addresses[1]
	encoded, err := json.Marshal(document)
	if err != nil {
		return nil, "", errors.New("CORE_CONFIG_INVALID")
	}
	parsed, err := executor.ParseWithBytes(encoded)
	if err != nil {
		return nil, "", errors.New("CORE_CONFIG_INVALID")
	}
	return parsed, compiled.FrontName, nil
}
