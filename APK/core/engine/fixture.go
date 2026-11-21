package engine

import (
	"bytes"
	"encoding/json"
	"errors"
	"io"
	"net/netip"

	"github.com/metacubex/mihomo/config"
	"github.com/metacubex/mihomo/hub/executor"
)

var errFixtureInvalid = errors.New("CORE_FIXTURE_INVALID")

type fixtureEndpoint struct {
	Server   string `json:"server"`
	Port     int    `json:"port"`
	Cipher   string `json:"cipher,omitempty"`
	Username string `json:"username,omitempty"`
	Password string `json:"password"`
}

type fixtureInput struct {
	Front fixtureEndpoint `json:"front"`
	Exit  fixtureEndpoint `json:"exit"`
}

func ValidateFixture(data []byte) error {
	coreMutex.Lock()
	defer coreMutex.Unlock()
	if session.id != 0 {
		return errors.New("CORE_SESSION_ACTIVE")
	}
	_, err := parseFixture(data)
	return err
}

// A0 accepts literal IPv4 endpoints to exclude unproven bootstrap DNS paths.
func parseFixture(data []byte) (*config.Config, error) {
	if len(data) > 16*1024 {
		return nil, errFixtureInvalid
	}
	var input fixtureInput
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.DisallowUnknownFields()
	if decoder.Decode(&input) != nil {
		return nil, errFixtureInvalid
	}
	if decoder.Decode(new(any)) != io.EOF {
		return nil, errFixtureInvalid
	}
	for _, endpoint := range []fixtureEndpoint{input.Front, input.Exit} {
		address, err := netip.ParseAddr(endpoint.Server)
		if err != nil || !address.Is4() || address.IsUnspecified() || address.IsMulticast() || endpoint.Port < 1 || endpoint.Port > 65535 || endpoint.Password == "" {
			return nil, errFixtureInvalid
		}
	}
	if input.Front.Cipher != "aes-128-gcm" || input.Front.Username != "" || input.Exit.Cipher != "" || input.Exit.Username == "" || len(input.Exit.Username) > 255 || len(input.Exit.Password) > 255 {
		return nil, errFixtureInvalid
	}
	configuration := map[string]any{
		"mode": "rule", "log-level": "silent", "ipv6": false,
		"find-process-mode": "off", "geo-auto-update": false,
		"profile": map[string]any{"store-selected": false, "store-fake-ip": false},
		"tun":     map[string]any{"enable": false},
		"dns": map[string]any{
			"enable": true, "ipv6": false, "enhanced-mode": "redir-host",
			"nameserver": []string{"https://1.1.1.1/dns-query#CHAIN-EXIT"},
		},
		"proxies": []map[string]any{
			{"name": "CHAIN-FRONT", "type": "ss", "server": input.Front.Server, "port": input.Front.Port, "cipher": input.Front.Cipher, "password": input.Front.Password, "udp": false},
			{"name": "CHAIN-EXIT", "type": "socks5", "server": input.Exit.Server, "port": input.Exit.Port, "username": input.Exit.Username, "password": input.Exit.Password, "udp": false, "dialer-proxy": "CHAIN-FRONT"},
		},
		"proxy-groups": []any{},
		"rules":        []string{"NETWORK,udp,REJECT", "MATCH,CHAIN-EXIT"},
	}
	encoded, err := json.Marshal(configuration)
	if err != nil {
		return nil, errFixtureInvalid
	}
	parsed, err := executor.ParseWithBytes(encoded)
	if err != nil {
		return nil, errFixtureInvalid
	}
	return parsed, nil
}
